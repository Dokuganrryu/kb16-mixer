//! The mixer engine. A channel is either a group of apps or the microphone; moving a channel
//! moves the same sliders the Windows volume mixer shows. Nothing is routed or captured, and
//! no other process is ever opened.
//!
//! The pad follows Windows, not the other way round: a slider moved in the Windows mixer or
//! inside an app is picked up on the next refresh. The one rule the engine adds is that an app
//! which starts making sound takes the level of the channel it lands in.

use crate::config::{ChannelCfg, ChannelTarget};
use crate::procs::{self, Snapshot};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::core::{GUID, Interface, Result};

const NO_CONTEXT: *const GUID = std::ptr::null();
const REFRESH_EVERY: Duration = Duration::from_millis(1500);
/// After the pad changes a channel, Windows is not read back for this long (sessions report
/// a new volume slightly late, which would make the knob stutter).
const SETTLE: Duration = Duration::from_millis(1200);

struct Session {
    pid: u32,
    exe: String,
    volume: ISimpleAudioVolume,
    meter: Option<IAudioMeterInformation>,
    channel: Option<usize>,
}

#[derive(Clone, Copy, Default)]
pub struct Fader {
    pub volume: f32,
    pub muted: bool,
    /// False until the level is known from a session, the saved state, or the user.
    pub known: bool,
    /// Set on the pad (now or in an earlier run). Only then do new apps take this level;
    /// a level merely read from some app is never pushed onto another.
    pub pinned: bool,
    touched: Option<Instant>,
}

/// What a knob or popup reports back.
pub struct Readout {
    pub name: String,
    pub volume: u8,
    pub muted: bool,
    /// False when the channel has nothing playing right now.
    pub live: bool,
}

#[derive(Clone, Copy)]
pub enum Device {
    Master,
    Mic,
}

/// Result of holding a channel key with an app in front.
pub enum Moved {
    To,
    BackToAuto,
    NothingFocused,
    NotAnAppChannel,
}

struct Mic {
    volume: IAudioEndpointVolume,
    meter: Option<IAudioMeterInformation>,
}

pub struct Mixer {
    devices: IMMDeviceEnumerator,
    sessions: Vec<Session>,
    exe_of: HashMap<u32, String>,
    mic: Option<Mic>,
    refreshed: Option<Instant>,
    pub faders: Vec<Fader>,
    /// Apps moved by hand: exe (lowercase) -> channel name. Beats the config's app lists.
    pub moved: HashMap<String, String>,
    /// Focus: this channel stands out, every other app channel plays at `duck` of its level.
    focus: Option<usize>,
    pub duck: f32,
    /// Real level of each session Focus has turned down, by pid.
    ducked: HashMap<u32, f32>,
    /// The same by exe name. Saved to disk while Focus is on; anything still in here when no
    /// Focus is active is left over from a run that ended mid-focus and gets its level back.
    pub restore: HashMap<String, f32>,
}

impl Mixer {
    pub fn new() -> Result<Mixer> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            Ok(Mixer {
                devices: CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?,
                sessions: Vec::new(),
                exe_of: HashMap::new(),
                mic: None,
                refreshed: None,
                faders: Vec::new(),
                moved: HashMap::new(),
                focus: None,
                duck: 0.25,
                ducked: HashMap::new(),
                restore: HashMap::new(),
            })
        }
    }

    /// Starts over for a (re)loaded config. `saved` holds remembered channel levels by name.
    pub fn reset(&mut self, channels: &[ChannelCfg], saved: &HashMap<String, f32>) {
        self.faders = channels
            .iter()
            .map(|c| match saved.get(&c.name) {
                Some(&v) => Fader { volume: v.clamp(0.0, 1.0), known: true, pinned: true, ..Fader::default() },
                None => Fader::default(),
            })
            .collect();
        self.sessions.clear();
        self.refreshed = None;
    }

    pub fn focus(&self) -> Option<usize> {
        self.focus
    }

    /// True if Focus is currently turning `channel` down.
    fn is_ducked(&self, channel: usize, channels: &[ChannelCfg]) -> bool {
        let is_mic = matches!(channels.get(channel).map(|c| &c.target), Some(ChannelTarget::Mic));
        self.focus.is_some_and(|f| f != channel) && !is_mic
    }

    /// Starts, moves or ends Focus. Sessions get their real level back before anything new
    /// is turned down, so levels never compound.
    pub fn set_focus(&mut self, focus: Option<usize>, channels: &[ChannelCfg]) {
        if self.focus == focus {
            return;
        }
        let now = Instant::now();
        unsafe {
            for s in &self.sessions {
                if let Some(&level) = self.ducked.get(&s.pid) {
                    let _ = s.volume.SetMasterVolume(level, NO_CONTEXT);
                }
            }
        }
        self.ducked.clear();
        self.restore.clear();
        self.focus = focus;
        for i in 0..self.sessions.len() {
            let Some(c) = self.sessions[i].channel else { continue };
            if !self.is_ducked(c, channels) {
                continue;
            }
            let s = &self.sessions[i];
            let level = unsafe { s.volume.GetMasterVolume().unwrap_or(1.0) };
            unsafe {
                let _ = s.volume.SetMasterVolume(level * self.duck, NO_CONTEXT);
            }
            self.ducked.insert(s.pid, level);
            self.restore.insert(s.exe.clone(), level);
        }
        // Sessions report the change a little late; do not read it back as a new level.
        for f in &mut self.faders {
            f.touched = Some(now);
        }
    }

    /// Re-reads Windows (at most every `REFRESH_EVERY` unless forced).
    pub fn refresh(&mut self, channels: &[ChannelCfg], force: bool) {
        if !force && self.refreshed.is_some_and(|t| t.elapsed() < REFRESH_EVERY) {
            return;
        }
        let first = self.refreshed.is_none();
        self.refreshed = Some(Instant::now());
        if self.faders.len() != channels.len() {
            self.faders.resize(channels.len(), Fader::default());
        }

        let before: HashSet<u32> = self.sessions.iter().map(|s| s.pid).collect();
        let mut sessions = self.enumerate();
        self.name_processes(&mut sessions);
        let needs_focus = channels.iter().any(|c| matches!(c.target, ChannelTarget::Focused));
        let focus = if needs_focus { procs::foreground_pid().map(|p| (p, Snapshot::take())) } else { None };
        for s in &mut sessions {
            s.channel = assign(channels, &self.moved, &s.exe, s.pid, focus.as_ref());
        }

        // Left over from a run that ended while Focus was on: give those apps their level back.
        if self.focus.is_none() && !self.restore.is_empty() {
            for s in &sessions {
                if let Some(level) = self.restore.remove(&s.exe) {
                    unsafe {
                        let _ = s.volume.SetMasterVolume(level, NO_CONTEXT);
                    }
                }
            }
        }

        // Sessions already playing tell us where Windows stands; new ones take the channel level.
        let mut adopted = vec![false; channels.len()];
        for s in &sessions {
            let Some(c) = s.channel else { continue };
            let ducked = self.is_ducked(c, channels);
            let f = &mut self.faders[c];
            let settled = f.touched.is_none_or(|t| t.elapsed() >= SETTLE);
            let is_new = !first && !before.contains(&s.pid);
            if ducked {
                // A turned-down level is ours, never Windows' truth. An app that appears
                // mid-focus is turned down too, remembering what it should be.
                if !self.ducked.contains_key(&s.pid) {
                    let level = if is_new && f.pinned {
                        f.volume
                    } else {
                        unsafe { s.volume.GetMasterVolume().unwrap_or(1.0) }
                    };
                    unsafe {
                        let _ = s.volume.SetMasterVolume(level * self.duck, NO_CONTEXT);
                    }
                    self.ducked.insert(s.pid, level);
                    self.restore.insert(s.exe.clone(), level);
                }
                continue;
            }
            unsafe {
                if is_new && (f.pinned || f.muted) {
                    // An app joining a channel takes its dialled-in level, and its mute.
                    if f.pinned {
                        let _ = s.volume.SetMasterVolume(f.volume, NO_CONTEXT);
                    }
                    if f.muted {
                        let _ = s.volume.SetMute(true, NO_CONTEXT);
                    }
                } else if settled && !adopted[c] {
                    f.volume = s.volume.GetMasterVolume().unwrap_or(f.volume);
                    f.muted = s.volume.GetMute().map(|b| b.as_bool()).unwrap_or(f.muted);
                    f.known = true;
                    adopted[c] = true;
                }
            }
        }
        let live: HashSet<u32> = sessions.iter().map(|s| s.pid).collect();
        self.ducked.retain(|pid, _| live.contains(pid));
        self.sessions = sessions;

        self.mic = self.open_mic();
        if let (Some(c), Some(mic)) = (mic_channel(channels), &self.mic) {
            let f = &mut self.faders[c];
            if f.touched.is_none_or(|t| t.elapsed() >= SETTLE) {
                unsafe {
                    f.volume = mic.volume.GetMasterVolumeLevelScalar().unwrap_or(f.volume);
                    f.muted = mic.volume.GetMute().map(|b| b.as_bool()).unwrap_or(f.muted);
                    f.known = true;
                }
            }
        }
    }

    /// Every live app session on every active output device.
    fn enumerate(&self) -> Vec<Session> {
        let mut out = Vec::new();
        unsafe {
            let Ok(coll) = self.devices.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return out };
            for i in 0..coll.GetCount().unwrap_or(0) {
                let Ok(dev) = coll.Item(i) else { continue };
                let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
                let Ok(list) = mgr.GetSessionEnumerator() else { continue };
                for j in 0..list.GetCount().unwrap_or(0) {
                    let Ok(ctl) = list.GetSession(j) else { continue };
                    if ctl.GetState().map_or(true, |s| s == AudioSessionStateExpired) {
                        continue;
                    }
                    let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
                    if ctl2.IsSystemSoundsSession() == S_OK {
                        continue;
                    }
                    let Ok(volume) = ctl.cast::<ISimpleAudioVolume>() else { continue };
                    out.push(Session {
                        pid: ctl2.GetProcessId().unwrap_or(0),
                        exe: String::new(),
                        volume,
                        meter: ctl.cast::<IAudioMeterInformation>().ok(),
                        channel: None,
                    });
                }
            }
        }
        out
    }

    /// Fills in exe names from a cache; the process list is only read when a new pid shows up.
    fn name_processes(&mut self, sessions: &mut [Session]) {
        if sessions.iter().any(|s| !self.exe_of.contains_key(&s.pid)) {
            let snap = Snapshot::take();
            for s in sessions.iter() {
                self.exe_of.insert(s.pid, snap.exe(s.pid).unwrap_or("").to_string());
            }
        }
        let live: HashSet<u32> = sessions.iter().map(|s| s.pid).collect();
        self.exe_of.retain(|pid, _| live.contains(pid));
        for s in sessions.iter_mut() {
            s.exe = self.exe_of.get(&s.pid).cloned().unwrap_or_default();
        }
    }

    /// The microphone chat apps use (the default communications capture device).
    fn open_mic(&self) -> Option<Mic> {
        unsafe {
            let dev = self
                .devices
                .GetDefaultAudioEndpoint(eCapture, eCommunications)
                .or_else(|_| self.devices.GetDefaultAudioEndpoint(eCapture, eConsole))
                .ok()?;
            Some(Mic {
                volume: dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()?,
                meter: dev.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None).ok(),
            })
        }
    }

    /// Live level per channel, 0..255 on a -48..0 dB scale.
    pub fn levels(&self, channels: &[ChannelCfg]) -> [u8; 4] {
        let mut peak = [0f32; 4];
        for s in &self.sessions {
            let (Some(c), Some(m)) = (s.channel, &s.meter) else { continue };
            if c < 4 {
                peak[c] = peak[c].max(unsafe { m.GetPeakValue().unwrap_or(0.0) });
            }
        }
        if let (Some(c), Some(Mic { meter: Some(m), .. })) = (mic_channel(channels), &self.mic) {
            if c < 4 {
                peak[c] = unsafe { m.GetPeakValue().unwrap_or(0.0) };
            }
        }
        peak.map(to_level)
    }

    fn is_live(&self, channel: usize, channels: &[ChannelCfg]) -> bool {
        match channels.get(channel).map(|c| &c.target) {
            Some(ChannelTarget::Mic) => self.mic.is_some(),
            _ => self.sessions.iter().any(|s| s.channel == Some(channel)),
        }
    }

    /// Moves a channel's fader by `delta_pct`, or toggles its mute when `delta_pct` is None.
    pub fn change_channel(&mut self, channel: usize, delta_pct: Option<f32>, channels: &[ChannelCfg]) -> Readout {
        let Some(cfg) = channels.get(channel) else {
            return Readout { name: String::new(), volume: 0, muted: false, live: false };
        };
        let f = &mut self.faders[channel];
        match delta_pct {
            Some(d) => {
                if !f.known {
                    f.volume = 1.0;
                }
                f.volume = (f.volume + d / 100.0).clamp(0.0, 1.0);
                // Turning a muted app channel up unmutes it. The mic never unmutes by accident.
                if d > 0.0 && !matches!(cfg.target, ChannelTarget::Mic) {
                    f.muted = false;
                }
                // Only a level the user dialled in is passed on to apps that start later.
                f.known = true;
                f.pinned = true;
            }
            None => f.muted = !f.muted,
        }
        f.touched = Some(Instant::now());
        let set_level = delta_pct.is_some();
        let f = *f;
        let ducked = self.is_ducked(channel, channels);
        unsafe {
            if matches!(cfg.target, ChannelTarget::Mic) {
                if let Some(mic) = &self.mic {
                    if set_level {
                        let _ = mic.volume.SetMasterVolumeLevelScalar(f.volume, NO_CONTEXT);
                    }
                    let _ = mic.volume.SetMute(f.muted, NO_CONTEXT);
                }
            } else {
                for s in self.sessions.iter().filter(|s| s.channel == Some(channel)) {
                    let _ = s.volume.SetMute(f.muted, NO_CONTEXT);
                    if !set_level {
                        continue;
                    }
                    let heard = if ducked { f.volume * self.duck } else { f.volume };
                    let _ = s.volume.SetMasterVolume(heard, NO_CONTEXT);
                    if ducked {
                        self.ducked.insert(s.pid, f.volume);
                        self.restore.insert(s.exe.clone(), f.volume);
                    }
                }
            }
        }
        self.channel_readout(channel, channels)
    }

    pub fn mic_muted(&self) -> bool {
        self.mic.as_ref().is_some_and(|m| unsafe { m.volume.GetMute().map(|b| b.as_bool()).unwrap_or(false) })
    }

    /// Mutes or unmutes the microphone, keeping a Mic channel's fader in step.
    pub fn set_mic_muted(&mut self, muted: bool, channels: &[ChannelCfg]) {
        if let Some(mic) = &self.mic {
            unsafe {
                let _ = mic.volume.SetMute(muted, NO_CONTEXT);
            }
        }
        if let Some(f) = mic_channel(channels).and_then(|c| self.faders.get_mut(c)) {
            f.muted = muted;
            f.touched = Some(Instant::now());
        }
    }

    /// Which channel the app in front belongs to, and its display name.
    pub fn where_is_focused(&self, channels: &[ChannelCfg]) -> Option<(String, Option<usize>)> {
        let snap = Snapshot::take();
        let fg = procs::foreground_pid()?;
        let exe = snap.exe(fg)?.to_string();
        if exe.is_empty() || exe == "explorer.exe" {
            return None;
        }
        let playing = self.sessions.iter().find(|s| s.exe == exe).and_then(|s| s.channel);
        Some((pretty(&exe), playing.or_else(|| assign(channels, &self.moved, &exe, fg, None))))
    }

    /// A channel's current fader, without changing it.
    pub fn channel_readout(&self, channel: usize, channels: &[ChannelCfg]) -> Readout {
        let f = self.faders.get(channel).copied().unwrap_or_default();
        Readout {
            name: channels.get(channel).map(|c| c.name.clone()).unwrap_or_default(),
            volume: pct(if f.known { f.volume } else { 1.0 }),
            muted: f.muted,
            live: self.is_live(channel, channels),
        }
    }

    /// The app in front, whatever channel it is in.
    pub fn change_focused(&mut self, delta_pct: Option<f32>) -> Readout {
        let snap = Snapshot::take();
        let Some(fg) = procs::foreground_pid() else {
            return Readout { name: "Nothing in front".into(), volume: 0, muted: false, live: false };
        };
        let fg_exe = snap.exe(fg).unwrap_or("").to_string();
        let name = pretty(&fg_exe);
        let hits: Vec<&Session> = self
            .sessions
            .iter()
            .filter(|s| snap.descends_from(s.pid, fg) || (!fg_exe.is_empty() && s.exe == fg_exe))
            .collect();
        let Some(first) = hits.first() else { return Readout { name, volume: 0, muted: false, live: false } };
        unsafe {
            let mut vol = first.volume.GetMasterVolume().unwrap_or(1.0);
            let mut muted = first.volume.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            match delta_pct {
                Some(d) => {
                    vol = (vol + d / 100.0).clamp(0.0, 1.0);
                    if d > 0.0 {
                        muted = false;
                    }
                }
                None => muted = !muted,
            }
            for s in &hits {
                let _ = s.volume.SetMasterVolume(vol, NO_CONTEXT);
                let _ = s.volume.SetMute(muted, NO_CONTEXT);
            }
            // Its channel's fader follows on the next refresh.
            self.refreshed = Some(Instant::now() - REFRESH_EVERY + Duration::from_millis(300));
            Readout { name, volume: pct(vol), muted, live: true }
        }
    }

    fn endpoint(&self, device: Device) -> Option<IAudioEndpointVolume> {
        unsafe {
            match device {
                Device::Mic => self.open_mic().map(|m| m.volume),
                Device::Master => {
                    let dev = self.devices.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
                    dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
                }
            }
        }
    }

    pub fn device_muted(&self, device: Device) -> bool {
        self.endpoint(device).is_some_and(|ep| unsafe { ep.GetMute().map(|b| b.as_bool()).unwrap_or(false) })
    }

    pub fn change_device(&mut self, device: Device, delta_pct: Option<f32>) -> Readout {
        let name = match device {
            Device::Master => "Master",
            Device::Mic => "Mic",
        };
        let Some(ep) = self.endpoint(device) else {
            return Readout { name: name.into(), volume: 0, muted: false, live: false };
        };
        unsafe {
            let mut vol = ep.GetMasterVolumeLevelScalar().unwrap_or(0.0);
            let mut muted = ep.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            match delta_pct {
                Some(d) => {
                    vol = (vol + d / 100.0).clamp(0.0, 1.0);
                    let _ = ep.SetMasterVolumeLevelScalar(vol, NO_CONTEXT);
                    if d > 0.0 && muted && matches!(device, Device::Master) {
                        muted = false;
                        let _ = ep.SetMute(false, NO_CONTEXT);
                    }
                }
                None => {
                    muted = !muted;
                    let _ = ep.SetMute(muted, NO_CONTEXT);
                }
            }
            Readout { name: name.into(), volume: pct(vol), muted, live: true }
        }
    }

    /// Moves the app in front to `channel`, or back to its automatic channel if it is
    /// already there by hand. Returns what happened and the app's display name.
    pub fn move_focused(&mut self, channel: usize, channels: &[ChannelCfg]) -> (Moved, String) {
        let Some(cfg) = channels.get(channel) else { return (Moved::NothingFocused, String::new()) };
        if matches!(cfg.target, ChannelTarget::Mic) {
            return (Moved::NotAnAppChannel, String::new());
        }
        let snap = Snapshot::take();
        let exe = procs::foreground_pid().and_then(|p| snap.exe(p)).unwrap_or("").to_string();
        if exe.is_empty() || exe == "explorer.exe" {
            return (Moved::NothingFocused, String::new());
        }
        let outcome = if self.moved.get(&exe).is_some_and(|c| c.eq_ignore_ascii_case(&cfg.name)) {
            self.moved.remove(&exe);
            Moved::BackToAuto
        } else {
            self.moved.insert(exe.clone(), cfg.name.clone());
            Moved::To
        };
        self.refresh(channels, true);
        // The app takes on the level of the channel it landed in.
        for s in self.sessions.iter().filter(|s| s.exe == exe) {
            let Some(f) = s.channel.and_then(|c| self.faders.get(c)).filter(|f| f.pinned) else { continue };
            unsafe {
                let _ = s.volume.SetMasterVolume(f.volume, NO_CONTEXT);
                let _ = s.volume.SetMute(f.muted, NO_CONTEXT);
            }
        }
        (outcome, pretty(&exe))
    }

    /// One line per session, for `--selftest`.
    pub fn describe(&self) -> Vec<String> {
        self.sessions
            .iter()
            .map(|s| unsafe {
                let v = s.volume.GetMasterVolume().unwrap_or(-1.0);
                format!("pid {:>6}  {:<26} channel {:?}  vol {:>3.0}%", s.pid, s.exe, s.channel, v * 100.0)
            })
            .collect()
    }
}

pub fn mic_channel(channels: &[ChannelCfg]) -> Option<usize> {
    channels.iter().position(|c| matches!(c.target, ChannelTarget::Mic))
}

/// An app moved by hand goes where it was put. Otherwise the first channel that names it
/// wins, and an "other" channel takes everything left over.
fn assign(
    channels: &[ChannelCfg],
    moved: &HashMap<String, String>,
    exe: &str,
    pid: u32,
    focus: Option<&(u32, Snapshot)>,
) -> Option<usize> {
    let by_name = |name: &String| {
        channels.iter().position(|c| c.name.eq_ignore_ascii_case(name) && !matches!(c.target, ChannelTarget::Mic))
    };
    if let Some(i) = moved.get(exe).and_then(by_name) {
        return Some(i);
    }
    let named = channels.iter().position(|c| match &c.target {
        ChannelTarget::Apps(apps) => apps.iter().any(|a| a.eq_ignore_ascii_case(exe)),
        ChannelTarget::Focused => focus.is_some_and(|(fg, snap)| snap.descends_from(pid, *fg)),
        ChannelTarget::Other | ChannelTarget::Mic => false,
    });
    named.or_else(|| channels.iter().position(|c| matches!(c.target, ChannelTarget::Other)))
}

fn to_level(peak: f32) -> u8 {
    if peak <= 0.0 {
        return 0;
    }
    let db = 20.0 * peak.log10();
    (((db + 48.0) / 48.0).clamp(0.0, 1.0) * 255.0) as u8
}

fn pct(v: f32) -> u8 {
    (v * 100.0).round() as u8
}

/// "discord.exe" -> "Discord"
pub fn pretty(exe: &str) -> String {
    let stem = exe.strip_suffix(".exe").unwrap_or(exe);
    let mut chars = stem.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
