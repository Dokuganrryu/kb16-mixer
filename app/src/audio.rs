//! The mixer engine. A channel is either a group of apps or the microphone; moving a channel
//! moves the same sliders the Windows volume mixer shows. Nothing is routed or captured, and
//! no other process is ever opened.
//!
//! An app is heard at `channel level x its trim` (see `trim.rs`). The rules, one each:
//! - Windows wins. A slider moved in the Windows mixer or inside an app is picked up on the
//!   next refresh and becomes that app's level through `trim::place`, exactly as if it had
//!   been turned on the pad: no other app changes what it sounds like.
//! - An app that starts making sound in a channel whose level was set on the pad takes
//!   `level x its remembered trim` (1 for an app never turned on its own), and the channel's
//!   mute. In a channel never set on the pad, its own level is followed instead.
//! - Mute keys mute the whole channel and never touch a level or a trim.
//! - Focus sits on top: it turns sessions down from their real level and puts back exactly
//!   that level; nothing is learned from Windows while a channel is turned down.

use crate::config::{ChannelCfg, ChannelTarget};
use crate::procs::{self, Snapshot};
use crate::trim::{self, place};
use std::collections::{BTreeSet, HashMap, HashSet};
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
/// A Windows level this far from what the engine expects was moved by someone else.
const MOVED: f32 = 0.005;
/// About -40 dBFS: quieter than this, an app does not count as playing.
const AUDIBLE: f32 = 0.01;
/// Sound must last this long to count (a notification ping does not)...
const AUDIBLE_RISE: Duration = Duration::from_millis(1000);
/// ...and silence this long to end it (the gap between two songs does not).
const AUDIBLE_FALL: Duration = Duration::from_millis(2500);

/// One session's slider in the Windows mixer. A trait so the rules above can be tested
/// without Core Audio.
trait Slider {
    fn level(&self) -> Option<f32>;
    fn set_level(&self, v: f32);
    fn muted(&self) -> Option<bool>;
    fn set_muted(&self, muted: bool);
}

impl Slider for ISimpleAudioVolume {
    fn level(&self) -> Option<f32> {
        // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the pointer
        // and the HRESULT.
        unsafe { self.GetMasterVolume().ok() }
    }
    fn set_level(&self, v: f32) {
        // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the pointer
        // and the HRESULT. No event context is passed.
        unsafe {
            let _ = self.SetMasterVolume(v, NO_CONTEXT);
        }
    }
    fn muted(&self) -> Option<bool> {
        // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the pointer
        // and the HRESULT.
        unsafe { self.GetMute().ok().map(|b| b.as_bool()) }
    }
    fn set_muted(&self, muted: bool) {
        // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the pointer
        // and the HRESULT. No event context is passed.
        unsafe {
            let _ = self.SetMute(muted, NO_CONTEXT);
        }
    }
}

struct Session {
    pid: u32,
    /// Windows' session instance id. One process can have several sessions, each with a
    /// slider of its own (one per output device, or per stream), so the pid does not name one.
    id: String,
    exe: String,
    volume: Box<dyn Slider>,
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
    /// The default output device is muted: nothing an app plays is heard.
    output_muted: bool,
    refreshed: Option<Instant>,
    pub faders: Vec<Fader>,
    /// Apps moved by hand: exe (lowercase) -> channel name. Beats the config's app lists.
    pub moved: HashMap<String, String>,
    /// Focus: this channel stands out, every other app channel plays at `duck` of its level.
    focus: Option<usize>,
    pub duck: f32,
    /// Real level of each session Focus has turned down, by session id.
    ducked: HashMap<String, f32>,
    /// The same by exe name (see `sync_restore`), plus the real level of apps that were turned
    /// down when they closed or when the last run ended: their slider may still show the
    /// turned-down level, so they get the real one back when they appear. Saved to disk.
    pub restore: HashMap<String, f32>,
    /// Each app's share of its channel level, by exe; missing = `trim::FULL`. Saved to disk.
    pub trims: HashMap<String, f32>,
}

impl Mixer {
    pub fn new() -> Result<Mixer> {
        // SAFETY: COM is initialized for this thread before the enumerator is created; both are
        // plain calls.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            Ok(Mixer {
                devices: CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?,
                sessions: Vec::new(),
                exe_of: HashMap::new(),
                mic: None,
                output_muted: false,
                refreshed: None,
                faders: Vec::new(),
                moved: HashMap::new(),
                focus: None,
                duck: 0.25,
                ducked: HashMap::new(),
                restore: HashMap::new(),
                trims: HashMap::new(),
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
        let real = std::mem::take(&mut self.ducked);
        for s in &self.sessions {
            if let Some(&level) = real.get(&s.id) {
                s.volume.set_level(level);
            }
        }
        self.focus = focus;
        for s in &self.sessions {
            if !s.channel.is_some_and(|c| self.is_ducked(c, channels)) {
                continue;
            }
            let Some((down, level)) = dip(real.get(&s.id).copied().or_else(|| s.volume.level()), self.duck) else {
                continue;
            };
            s.volume.set_level(down);
            self.ducked.insert(s.id.clone(), level);
        }
        self.sync_restore();
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

        let mut sessions = self.enumerate();
        self.name_processes(&mut sessions);
        let needs_focus = channels.iter().any(|c| matches!(c.target, ChannelTarget::Focused));
        let focus = if needs_focus { procs::foreground_pid().map(|p| (p, Snapshot::take())) } else { None };
        for s in &mut sessions {
            s.channel = assign(channels, &self.moved, &s.exe, s.pid, focus.as_ref());
        }
        self.follow(sessions, channels, first);

        self.mic = self.open_mic();
        self.output_muted = self.device_muted(Device::Master);
        if let (Some(c), Some(mic)) = (mic_channel(channels), &self.mic) {
            let f = &mut self.faders[c];
            if f.touched.is_none_or(|t| t.elapsed() >= SETTLE) {
                // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the
                // pointer and the HRESULT.
                unsafe {
                    f.volume = mic.volume.GetMasterVolumeLevelScalar().unwrap_or(f.volume);
                    f.muted = mic.volume.GetMute().map(|b| b.as_bool()).unwrap_or(f.muted);
                    f.known = true;
                }
            }
        }
    }

    /// Takes in the sessions Windows has now, each with its channel, by the rules at the top.
    fn follow(&mut self, sessions: Vec<Session>, channels: &[ChannelCfg], first: bool) {
        let now = Instant::now();
        let before: HashSet<String> = self.sessions.iter().map(|s| s.id.clone()).collect();
        let left_over = self.left_over();
        // Sessions already playing tell us where Windows stands; new ones take the channel level.
        let mut seen = Seen { adopted: vec![false; channels.len()], ..Seen::default() };
        for s in &sessions {
            let ducked = s.channel.is_some_and(|c| self.is_ducked(c, channels));
            let left = if self.ducked.contains_key(&s.id) { None } else { left_over.get(&s.exe).copied() };
            if !ducked && let Some(level) = self.ducked.remove(&s.id).or(left) {
                // No longer turned down (Focus ended while the app was gone, or the session left
                // a turned-down channel), but its slider still shows the reduced level: put the
                // real one back, and learn nothing until Windows reports it.
                s.volume.set_level(level);
                if let Some(f) = s.channel.and_then(|c| self.faders.get_mut(c)) {
                    f.touched = Some(now);
                }
                continue;
            }
            let Some(c) = s.channel else { continue };
            let is_new = !first && !before.contains(&s.id);
            if ducked {
                self.hold_down(s, c, left, is_new);
            } else {
                self.follow_session(s, c, is_new, &sessions, &mut seen);
            }
        }
        // A channel that was off came back on through one app: the rest return at their balance.
        for c in seen.came_back.iter().copied().filter(|&c| !self.is_ducked(c, channels)) {
            let level = self.faders[c].volume;
            for s in sessions.iter().filter(|s| s.channel == Some(c) && !seen.followed.contains(s.exe.as_str())) {
                s.volume.set_level(trim::heard(level, self.trim(&s.exe)));
            }
        }
        let live: HashSet<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        self.ducked.retain(|id, _| live.contains(id.as_str()));
        self.sessions = sessions;
        self.sync_restore();
    }

    /// Real levels no live turned-down session stands for: an app that closed while it was
    /// turned down, or one from a run that ended mid-focus (a crash, a power cut).
    fn left_over(&self) -> HashMap<String, f32> {
        let held: HashSet<&str> =
            self.sessions.iter().filter(|s| self.ducked.contains_key(&s.id)).map(|s| s.exe.as_str()).collect();
        self.restore.iter().filter(|(exe, _)| !held.contains(exe.as_str())).map(|(e, l)| (e.clone(), *l)).collect()
    }

    /// A session in a channel Focus turns down. A turned-down level is ours, never Windows'
    /// truth. An app that appears mid-focus is turned down too, remembering what it should be.
    /// One whose level cannot be read yet is tried again on the next refresh (see `dip`).
    fn hold_down(&mut self, s: &Session, c: usize, left: Option<f32>, is_new: bool) {
        if self.ducked.contains_key(&s.id) {
            return;
        }
        let f = &self.faders[c];
        let real = match left {
            Some(level) => Some(level),
            None if is_new && f.pinned => Some(trim::heard(f.volume, self.trim(&s.exe))),
            None => s.volume.level(),
        };
        let Some((down, level)) = dip(real, self.duck) else { return };
        s.volume.set_level(down);
        self.ducked.insert(s.id.clone(), level);
    }

    /// A session in a channel at its own level: a new one takes the channel's level, and a
    /// slider moved in Windows becomes the app's level (`trim::place`).
    fn follow_session<'a>(
        &mut self,
        s: &'a Session,
        c: usize,
        is_new: bool,
        sessions: &[Session],
        seen: &mut Seen<'a>,
    ) {
        let t = self.trim(&s.exe);
        let f = &mut self.faders[c];
        if is_new && f.muted {
            s.volume.set_muted(true);
        }
        if is_new && f.pinned {
            // An app joining a channel takes its dialled-in level times its trim.
            s.volume.set_level(trim::heard(f.volume, t));
            return;
        }
        if f.touched.is_some_and(|t| t.elapsed() < SETTLE) {
            return;
        }
        if !seen.adopted[c] && !is_new {
            f.muted = s.volume.muted().unwrap_or(f.muted);
            seen.adopted[c] = true;
        }
        let Some(v) = s.volume.level() else { return };
        if !f.known {
            // First sight of a channel nobody has set: its level follows this app.
            f.volume = if t >= MOVED { (v / t).min(1.0) } else { v };
            f.known = true;
        }
        // Several sessions of one app follow the first that moved.
        if (v - trim::heard(f.volume, t)).abs() < MOVED || !seen.followed.insert(&s.exe) {
            return;
        }
        let placed = place(f.volume, v);
        if f.volume < MOVED && placed.level >= MOVED {
            seen.came_back.insert(c);
        }
        f.volume = placed.level;
        self.trims.insert(s.exe.clone(), placed.trim);
        if placed.others != 1.0 {
            let others = sessions.iter().filter(|o| o.channel == Some(c) && o.exe != s.exe);
            scale_trims(&mut self.trims, others.map(|o| o.exe.as_str()), placed.others);
        }
        for twin in sessions.iter().filter(|o| o.exe == s.exe && o.id != s.id) {
            twin.volume.set_level(v);
        }
    }

    /// Rebuilds `restore` from `ducked`. An app that is not running keeps its entry, so it gets
    /// its real level back when it returns, whether or not Focus is still on then.
    fn sync_restore(&mut self) {
        let live: HashSet<&str> = self.sessions.iter().map(|s| s.exe.as_str()).collect();
        self.restore.retain(|exe, _| !live.contains(exe.as_str()));
        for s in &self.sessions {
            if let Some(&level) = self.ducked.get(&s.id) {
                self.restore.insert(s.exe.clone(), level);
            }
        }
    }

    /// Every live app session on every active output device.
    fn enumerate(&self) -> Vec<Session> {
        let mut out = Vec::new();
        // SAFETY: COM calls on the enumerator and the objects it returns. The instance id string is
        // freed once, after it was copied.
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
                    let pid = ctl2.GetProcessId().unwrap_or(0);
                    let id = match ctl2.GetSessionInstanceIdentifier() {
                        Ok(p) => {
                            let id = p.to_string().unwrap_or_default();
                            CoTaskMemFree(Some(p.0 as *const _));
                            id
                        }
                        Err(_) => String::new(),
                    };
                    out.push(Session {
                        pid,
                        id: if id.is_empty() { format!("{pid}/{i}/{j}") } else { id },
                        exe: String::new(),
                        volume: Box::new(volume),
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
        // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the pointer
        // and the HRESULT.
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

    /// Live level per channel for the meters, 0..255 on a -48..0 dB scale, and the loudest
    /// app as it is really heard. Windows reports a session's peak before its volume and mute,
    /// so the meters show what an app sends (and the Mic column the voice), while the second
    /// value applies both, and is 0 with the output device muted.
    pub fn levels(&self, channels: &[ChannelCfg]) -> ([u8; 4], f32) {
        let mut peak = [0f32; 4];
        let mut heard = 0f32;
        for s in &self.sessions {
            let Some(m) = &s.meter else { continue };
            // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the
            // pointer and the HRESULT.
            let p = unsafe { m.GetPeakValue().unwrap_or(0.0) };
            if let Some(c) = s.channel.filter(|&c| c < 4) {
                peak[c] = peak[c].max(p);
            }
            // Only a session that sends sound is asked for its slider.
            if p > AUDIBLE {
                let level = s.volume.level().unwrap_or(1.0);
                heard = heard.max(heard_peak(p, level, s.volume.muted().unwrap_or(false), self.output_muted));
            }
        }
        if let (Some(c), Some(Mic { meter: Some(m), .. })) = (mic_channel(channels), &self.mic)
            && c < 4
        {
            // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the
            // pointer and the HRESULT.
            peak[c] = unsafe { m.GetPeakValue().unwrap_or(0.0) };
        }
        (peak.map(to_level), heard)
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
        // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the pointer
        // and the HRESULT. No event context is passed.
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
                    s.volume.set_muted(f.muted);
                    if !set_level {
                        continue;
                    }
                    let level = trim::heard(f.volume, self.trims.get(&s.exe).copied().unwrap_or(trim::FULL));
                    s.volume.set_level(if ducked { level * self.duck } else { level });
                    if ducked {
                        self.ducked.insert(s.id.clone(), level);
                    }
                }
            }
        }
        self.sync_restore();
        self.channel_readout(channel, channels)
    }

    fn trim(&self, exe: &str) -> f32 {
        self.trims.get(exe).copied().unwrap_or(trim::FULL)
    }

    /// The apps making sound in a channel right now, by exe, in a stable order.
    pub fn apps_in(&self, channel: usize) -> Vec<String> {
        let mut apps: Vec<String> = self
            .sessions
            .iter()
            .filter(|s| s.channel == Some(channel) && !s.exe.is_empty())
            .map(|s| s.exe.clone())
            .collect();
        apps.sort();
        apps.dedup();
        apps
    }

    /// One app inside a channel, without changing it.
    pub fn app_readout(&self, channel: usize, exe: &str) -> Readout {
        let f = self.faders.get(channel).copied().unwrap_or_default();
        let level = if f.known { f.volume } else { 1.0 };
        Readout { name: pretty(exe), volume: pct(trim::heard(level, self.trim(exe))), muted: f.muted, live: true }
    }

    /// Turns one app in a channel by `delta_pct`; the rest of the channel sounds as before
    /// (see `trim::place`). Mute is left as it is: it belongs to the whole channel.
    pub fn change_app(&mut self, channel: usize, exe: &str, delta_pct: f32, channels: &[ChannelCfg]) -> Readout {
        let t = self.trim(exe);
        let ducked = self.is_ducked(channel, channels);
        let Some(f) = self.faders.get_mut(channel) else { return self.app_readout(channel, exe) };
        if !f.known {
            f.volume = 1.0;
        }
        let was_off = f.volume < MOVED;
        let v = (trim::heard(f.volume, t) + delta_pct / 100.0).clamp(0.0, 1.0);
        let placed = place(f.volume, v);
        f.volume = placed.level;
        f.known = true;
        f.pinned = true;
        f.touched = Some(Instant::now());
        let level = f.volume;
        self.trims.insert(exe.to_string(), placed.trim);
        if placed.others != 1.0 {
            let others = self.sessions.iter().filter(|s| s.channel == Some(channel) && s.exe != exe);
            scale_trims(&mut self.trims, others.map(|s| s.exe.as_str()), placed.others);
        }
        for s in self.sessions.iter().filter(|s| s.channel == Some(channel)) {
            // Only a channel coming back from off moves the others (to their old balance).
            let target = if s.exe == exe {
                v
            } else if was_off {
                trim::heard(level, self.trim(&s.exe))
            } else {
                continue;
            };
            s.volume.set_level(if ducked { target * self.duck } else { target });
            if ducked {
                self.ducked.insert(s.id.clone(), target);
            }
        }
        self.sync_restore();
        self.app_readout(channel, exe)
    }

    pub fn mic_muted(&self) -> bool {
        // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the pointer
        // and the HRESULT.
        self.mic.as_ref().is_some_and(|m| unsafe { m.volume.GetMute().map(|b| b.as_bool()).unwrap_or(false) })
    }

    /// Mutes or unmutes the microphone, keeping a Mic channel's fader in step.
    pub fn set_mic_muted(&mut self, muted: bool, channels: &[ChannelCfg]) {
        if let Some(mic) = &self.mic {
            // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the
            // pointer and the HRESULT. No event context is passed.
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
        let exe = procs::app_in_front(&snap, fg)?.to_string();
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
        self.change_in_front(&Snapshot::take(), procs::foreground_pid(), delta_pct)
    }

    /// `change_focused` with the process list and the pid in front given.
    fn change_in_front(&mut self, snap: &Snapshot, fg: Option<u32>, delta_pct: Option<f32>) -> Readout {
        let Some((fg, fg_exe)) = fg.and_then(|fg| Some((fg, procs::app_in_front(snap, fg)?))) else {
            return Readout { name: "Nothing in front".into(), volume: 0, muted: false, live: false };
        };
        let name = pretty(fg_exe);
        let hits: Vec<usize> = (0..self.sessions.len())
            .filter(|&i| {
                let s = &self.sessions[i];
                snap.descends_from(s.pid, fg) || s.exe == fg_exe
            })
            .collect();
        if hits.is_empty() {
            return Readout { name, volume: 0, muted: false, live: false };
        }
        let (vol, muted) = self.turn_sessions(&hits, delta_pct);
        // Its channel's fader follows on the next refresh.
        self.refreshed = Some(Instant::now() - REFRESH_EVERY + Duration::from_millis(300));
        Readout { name, volume: pct(vol), muted, live: true }
    }

    /// Turns these sessions together by `delta_pct` from the first one's level, or toggles their
    /// mute when it is None. Returns the new level and mute. A session Focus has turned down is
    /// turned from its real level, which it plays at times the duck and gets back after Focus.
    fn turn_sessions(&mut self, hits: &[usize], delta_pct: Option<f32>) -> (f32, bool) {
        let first = &self.sessions[hits[0]];
        let mut vol = self.ducked.get(&first.id).copied().or_else(|| first.volume.level()).unwrap_or(1.0);
        let mut muted = first.volume.muted().unwrap_or(false);
        match delta_pct {
            Some(d) => {
                vol = (vol + d / 100.0).clamp(0.0, 1.0);
                if d > 0.0 {
                    muted = false;
                }
            }
            None => muted = !muted,
        }
        for &i in hits {
            let s = &self.sessions[i];
            match self.ducked.get_mut(&s.id) {
                Some(real) => {
                    *real = vol;
                    s.volume.set_level(vol * self.duck);
                }
                None => s.volume.set_level(vol),
            }
            s.volume.set_muted(muted);
        }
        self.sync_restore();
        (vol, muted)
    }

    fn endpoint(&self, device: Device) -> Option<IAudioEndpointVolume> {
        // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the pointer
        // and the HRESULT.
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
        // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the pointer
        // and the HRESULT.
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
        // SAFETY: COM call on an interface this mixer holds; the windows crate wraps the pointer
        // and the HRESULT. No event context is passed.
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
        let Some(exe) = procs::foreground_pid().and_then(|p| procs::app_in_front(&snap, p)).map(str::to_string) else {
            return (Moved::NothingFocused, String::new());
        };
        let outcome = if self.moved.get(&exe).is_some_and(|c| c.eq_ignore_ascii_case(&cfg.name)) {
            self.moved.remove(&exe);
            Moved::BackToAuto
        } else {
            self.moved.insert(exe.clone(), cfg.name.clone());
            Moved::To
        };
        self.refresh(channels, true);
        // The app takes on the level of the channel it landed in, times its own trim (turned
        // down with the rest of that channel if Focus is on another one).
        let t = self.trim(&exe);
        for s in self.sessions.iter().filter(|s| s.exe == exe) {
            let Some(c) = s.channel else { continue };
            let Some(f) = self.faders.get(c).filter(|f| f.pinned) else { continue };
            let level = trim::heard(f.volume, t);
            if self.is_ducked(c, channels) {
                s.volume.set_level(level * self.duck);
                self.ducked.insert(s.id.clone(), level);
            } else {
                s.volume.set_level(level);
            }
            s.volume.set_muted(f.muted);
        }
        self.sync_restore();
        (outcome, pretty(&exe))
    }

    /// Every app with a volume slider now, by exe (lowercase), sorted.
    pub fn app_names(&self) -> Vec<String> {
        let mut apps: Vec<String> = self.sessions.iter().filter(|s| !s.exe.is_empty()).map(|s| s.exe.clone()).collect();
        apps.sort();
        apps.dedup();
        apps
    }

    /// One line per session, for `--selftest`.
    pub fn describe(&self) -> Vec<String> {
        self.sessions
            .iter()
            .map(|s| {
                let v = s.volume.level().unwrap_or(-1.0);
                format!("pid {:>6}  {:<26} channel {:?}  vol {:>3.0}%", s.pid, s.exe, s.channel, v * 100.0)
            })
            .collect()
    }

    /// What `--check` reports, as counts only: no session or device is named.
    pub fn census(&self) -> Census {
        // SAFETY: COM calls on the enumerator this mixer created; nothing is written.
        let (outputs, default_output) = unsafe {
            let outputs = self.devices.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE).and_then(|c| c.GetCount());
            (outputs.unwrap_or(0) as usize, self.devices.GetDefaultAudioEndpoint(eRender, eConsole).is_ok())
        };
        Census {
            outputs,
            default_output,
            output_muted: self.device_muted(Device::Master),
            sessions: self.enumerate().len(),
            mic: self.open_mic().is_some(),
        }
    }
}

/// Core Audio as `--check` sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Census {
    /// Active output devices.
    pub outputs: usize,
    pub default_output: bool,
    pub output_muted: bool,
    /// App sessions on all active output devices (system sounds not counted).
    pub sessions: usize,
    pub mic: bool,
}

pub fn mic_channel(channels: &[ChannelCfg]) -> Option<usize> {
    channels.iter().position(|c| matches!(c.target, ChannelTarget::Mic))
}

/// An app moved by hand goes where it was put. Otherwise the first channel that names it
/// wins, and an "other" channel takes everything left over. `exe` is lowercase. The settings
/// window shows the channels through this same rule (with no app in front).
pub fn assign(
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
        ChannelTarget::Focused => {
            focus.is_some_and(|(fg, snap)| procs::app_in_front(snap, *fg).is_some() && snap.descends_from(pid, *fg))
        }
        ChannelTarget::Other | ChannelTarget::Mic => false,
    });
    named.or_else(|| channels.iter().position(|c| matches!(c.target, ChannelTarget::Other)))
}

/// What one `follow` pass has learned so far, per channel.
#[derive(Default)]
struct Seen<'a> {
    /// The channel's mute was read from its first settled session.
    adopted: Vec<bool>,
    /// Apps whose moved slider was taken in (the first session of each app).
    followed: HashSet<&'a str>,
    /// Channels that were off and came back on through one app.
    came_back: BTreeSet<usize>,
}

/// Whether programme audio is playing, for the pad's idle timers.
#[derive(Default)]
pub struct Audible {
    on: bool,
    /// When the input started to disagree with `on`.
    since: Option<Instant>,
}

impl Audible {
    /// Feeds the loudest heard app (from `Mixer::levels`); returns the debounced answer.
    pub fn update(&mut self, heard: f32, now: Instant) -> bool {
        let loud = heard >= AUDIBLE;
        if loud == self.on {
            self.since = None;
        } else {
            let since = *self.since.get_or_insert(now);
            if now.duration_since(since) >= if loud { AUDIBLE_RISE } else { AUDIBLE_FALL } {
                self.on = loud;
                self.since = None;
            }
        }
        self.on
    }
}

/// How loud a session is really heard: its peak after its own slider and mute, and nothing
/// with the default output device muted. A peak below `AUDIBLE` counts as silence.
fn heard_peak(peak: f32, level: f32, muted: bool, output_muted: bool) -> f32 {
    if peak > AUDIBLE && !muted && !output_muted { peak * level } else { 0.0 }
}

/// What Focus does to one session whose real level is `real`: the level to set it to, and the
/// level to remember and put back later. None when the real level is unknown (the read failed):
/// the session then plays on as it is. Turning it down anyway would mean putting back a guess
/// (100 % would be loud), and an app left alone is the smaller fault.
fn dip(real: Option<f32>, duck: f32) -> Option<(f32, f32)> {
    real.map(|level| (level * duck, level))
}

/// Multiplies each app's trim once, however many sessions it has.
fn scale_trims<'a>(trims: &mut HashMap<String, f32>, apps: impl Iterator<Item = &'a str>, factor: f32) {
    let apps: HashSet<&str> = apps.collect();
    for app in apps {
        *trims.entry(app.to_string()).or_insert(trim::FULL) *= factor;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A slider in a pretend Windows mixer; clones share it, as two looks at one session do.
    /// Reading it can be made to fail, as a Core Audio call can.
    #[derive(Clone)]
    struct Fake(Rc<Cell<f32>>, Rc<Cell<bool>>, Rc<Cell<bool>>);

    impl Fake {
        fn at(level: f32) -> Fake {
            Fake(Rc::new(Cell::new(level)), Rc::new(Cell::new(true)), Rc::new(Cell::new(false)))
        }
        fn is_muted(&self) -> bool {
            self.2.get()
        }
        fn get(&self) -> f32 {
            self.0.get()
        }
        fn readable(&self, ok: bool) {
            self.1.set(ok);
        }
    }

    impl Slider for Fake {
        fn level(&self) -> Option<f32> {
            self.1.get().then(|| self.0.get())
        }
        fn set_level(&self, v: f32) {
            self.0.set(v);
        }
        fn muted(&self) -> Option<bool> {
            Some(self.2.get())
        }
        fn set_muted(&self, muted: bool) {
            self.2.set(muted);
        }
    }

    const MEDIA: usize = 0;
    const GAME: usize = 1;

    fn channels() -> Vec<ChannelCfg> {
        let ch = |name: &str, target| ChannelCfg { name: name.into(), color: [0; 3], target };
        vec![ch("Media", ChannelTarget::Apps(vec!["spotify.exe".into()])), ch("Game", ChannelTarget::Other)]
    }

    /// A mixer whose channels were set on the pad to these levels, with no app seen yet.
    fn mixer(levels: [f32; 2]) -> Mixer {
        let mut m = Mixer::new().expect("COM");
        m.faders =
            levels.iter().map(|&volume| Fader { volume, known: true, pinned: true, ..Fader::default() }).collect();
        m
    }

    /// What Windows lists now: (session id, exe, its slider, its channel). One process each.
    fn windows_has(sessions: &[(&str, &str, &Fake, usize)]) -> Vec<Session> {
        sessions
            .iter()
            .map(|&(id, exe, fake, channel)| Session {
                pid: 7,
                id: id.into(),
                exe: exe.into(),
                volume: Box::new(fake.clone()),
                meter: None,
                channel: Some(channel),
            })
            .collect()
    }

    /// One refresh, long enough after the last change for Windows to be read back.
    fn settled_refresh(m: &mut Mixer, sessions: Vec<Session>, first: bool) {
        for f in &mut m.faders {
            f.touched = None;
        }
        m.follow(sessions, &channels(), first);
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn every_session_of_an_app_follows_the_one_that_moved() {
        // Spotify plays on two output devices: one process, two sliders.
        let (a, b) = (Fake::at(0.5), Fake::at(0.5));
        let spotify = || windows_has(&[("a", "spotify.exe", &a, MEDIA), ("b", "spotify.exe", &b, MEDIA)]);
        let mut m = mixer([0.5, 0.5]);
        settled_refresh(&mut m, spotify(), true);
        a.set_level(0.8); // moved in the Windows mixer
        settled_refresh(&mut m, spotify(), false);
        assert!(close(b.get(), 0.8), "the other session follows: {}", b.get());
        for _ in 0..3 {
            settled_refresh(&mut m, spotify(), false);
        }
        assert!(close(m.faders[MEDIA].volume, 0.8) && m.trim("spotify.exe") == trim::FULL, "and it stays put");
        assert!(close(a.get(), 0.8) && close(b.get(), 0.8));
    }

    #[test]
    fn a_second_session_that_starts_during_focus_is_turned_down_too() {
        let (a, b) = (Fake::at(0.5), Fake::at(0.5));
        let mut m = mixer([0.5, 0.5]);
        settled_refresh(&mut m, windows_has(&[("a", "spotify.exe", &a, MEDIA)]), true);
        m.set_focus(Some(GAME), &channels());
        assert!(close(a.get(), 0.125));
        settled_refresh(
            &mut m,
            windows_has(&[("a", "spotify.exe", &a, MEDIA), ("b", "spotify.exe", &b, MEDIA)]),
            false,
        );
        assert!(close(b.get(), 0.125), "the new one is turned down: {}", b.get());
        m.set_focus(None, &channels());
        assert!(close(a.get(), 0.5) && close(b.get(), 0.5));
    }

    #[test]
    fn sessions_of_one_app_get_their_own_level_back() {
        let (a, b) = (Fake::at(0.4), Fake::at(0.8));
        let mut m = mixer([0.8, 0.5]);
        m.faders[MEDIA].touched = Some(Instant::now()); // just turned: Windows is not read back
        m.follow(windows_has(&[("a", "spotify.exe", &a, MEDIA), ("b", "spotify.exe", &b, MEDIA)]), &channels(), true);
        m.set_focus(Some(GAME), &channels());
        assert!(close(a.get(), 0.1) && close(b.get(), 0.2));
        m.set_focus(None, &channels());
        assert!(close(a.get(), 0.4) && close(b.get(), 0.8), "{} {}", a.get(), b.get());
    }

    #[test]
    fn an_app_closed_during_focus_comes_back_at_its_real_level() {
        let a = Fake::at(0.6);
        let mut m = mixer([0.5, 0.5]);
        m.faders[MEDIA] = Fader::default(); // never set on the pad: it follows the app
        settled_refresh(&mut m, windows_has(&[("a", "spotify.exe", &a, MEDIA)]), true);
        assert!(close(m.faders[MEDIA].volume, 0.6));
        m.set_focus(Some(GAME), &channels());
        settled_refresh(&mut m, Vec::new(), false); // Spotify is closed
        m.set_focus(None, &channels());
        // Windows remembers the turned-down level for the app's next session.
        let again = Fake::at(0.15);
        settled_refresh(&mut m, windows_has(&[("a2", "spotify.exe", &again, MEDIA)]), false);
        assert!(close(again.get(), 0.6), "its real level is put back: {}", again.get());
        settled_refresh(&mut m, windows_has(&[("a2", "spotify.exe", &again, MEDIA)]), false);
        assert!(close(m.faders[MEDIA].volume, 0.6), "and nothing is learned from the turned-down one");
        assert!(m.restore.is_empty());
    }

    #[test]
    fn a_session_that_leaves_a_turned_down_channel_gets_its_real_level() {
        let x = Fake::at(0.5);
        let mut m = mixer([0.5, 0.9]);
        settled_refresh(&mut m, windows_has(&[("x", "spotify.exe", &x, MEDIA)]), true);
        m.set_focus(Some(GAME), &channels());
        assert!(close(x.get(), 0.125));
        // Moved into the channel that stands out (the Move key, or it is now the app in front).
        settled_refresh(&mut m, windows_has(&[("x", "spotify.exe", &x, GAME)]), false);
        assert!(close(x.get(), 0.5), "real level back: {}", x.get());
        assert!(close(m.faders[GAME].volume, 0.9) && m.trim("spotify.exe") == trim::FULL, "nothing learned");
    }

    #[test]
    fn focus_turns_down_only_a_level_it_knows() {
        assert_eq!(dip(Some(0.8), 0.25), Some((0.2, 0.8)), "set to 0.2, put back 0.8");
        assert_eq!(dip(None, 0.25), None, "a failed read: nothing is set or remembered");
    }

    #[test]
    fn a_session_whose_level_cannot_be_read_is_not_turned_down_until_it_can() {
        let a = Fake::at(0.4);
        let mut m = mixer([0.5, 0.5]);
        m.faders[MEDIA].touched = Some(Instant::now()); // nothing learned from this refresh
        m.follow(windows_has(&[("a", "spotify.exe", &a, MEDIA)]), &channels(), true);
        a.readable(false);
        m.set_focus(Some(GAME), &channels());
        assert!(close(a.get(), 0.4), "left as it is: {}", a.get());
        assert!(m.restore.is_empty(), "no guess is remembered: {:?}", m.restore);
        settled_refresh(&mut m, windows_has(&[("a", "spotify.exe", &a, MEDIA)]), false);
        assert!(close(a.get(), 0.4) && m.restore.is_empty(), "still unknown, still left alone");
        a.readable(true);
        settled_refresh(&mut m, windows_has(&[("a", "spotify.exe", &a, MEDIA)]), false);
        assert!(close(a.get(), 0.1), "turned down once it is read: {}", a.get());
        m.set_focus(None, &channels());
        assert!(close(a.get(), 0.4), "and its real level comes back, not 100 %: {}", a.get());
    }

    #[test]
    fn crash_recovery_gives_every_session_of_an_app_its_level() {
        let (a, b) = (Fake::at(0.175), Fake::at(0.175));
        let mut m = mixer([0.7, 0.5]);
        m.restore.insert("spotify.exe".into(), 0.7); // the last run ended mid-focus
        settled_refresh(&mut m, windows_has(&[("a", "spotify.exe", &a, MEDIA), ("b", "spotify.exe", &b, MEDIA)]), true);
        assert!(close(a.get(), 0.7) && close(b.get(), 0.7), "{} {}", a.get(), b.get());
        assert!(m.restore.is_empty());
    }

    #[test]
    fn the_app_in_front_knob_moves_the_real_level_under_focus() {
        let a = Fake::at(0.6);
        let mut m = mixer([0.6, 0.5]);
        settled_refresh(&mut m, windows_has(&[("a", "spotify.exe", &a, MEDIA)]), true);
        m.set_focus(Some(GAME), &channels());
        let (vol, _) = m.turn_sessions(&[0], Some(10.0));
        assert!(close(vol, 0.7), "the readout shows the real level: {vol}");
        assert!(close(a.get(), 0.175), "still turned down: {}", a.get());
        m.set_focus(None, &channels());
        assert!(close(a.get(), 0.7), "the change survives Focus: {}", a.get());
    }

    #[test]
    fn an_app_goes_where_it_was_moved_then_to_the_first_list_that_names_it_then_to_other() {
        let ch = |name: &str, target| ChannelCfg { name: name.into(), color: [0; 3], target };
        let apps = |list: &[&str]| ChannelTarget::Apps(list.iter().map(|a| a.to_string()).collect());
        let channels = vec![
            ch("Mic", ChannelTarget::Mic),
            ch("Chat", apps(&["Discord.exe"])),
            ch("Game", ChannelTarget::Other),
            ch("Media", apps(&["spotify.exe", "discord.exe"])),
        ];
        let moved: HashMap<String, String> = [("spotify.exe", "game"), ("game.exe", "Mic"), ("old.exe", "Gone")]
            .map(|(e, c)| (e.into(), c.into()))
            .into();
        let at = |exe: &str| assign(&channels, &moved, exe, 1, None);
        assert_eq!(at("spotify.exe"), Some(2), "moved by hand beats the lists, by name in any case");
        assert_eq!(at("discord.exe"), Some(1), "the first list that names it, in any case");
        assert_eq!(at("game.exe"), Some(2), "never moved into the microphone");
        assert_eq!(at("old.exe"), Some(2), "moved to a channel that is gone: the usual rule");
        assert_eq!(at("anything.exe"), Some(2), "the rest go to Other");
        assert_eq!(assign(&channels[..2], &moved, "anything.exe", 1, None), None, "no Other: nowhere");
    }

    #[test]
    fn a_channel_turned_under_focus_plays_down_and_keeps_its_new_real_level() {
        let (a, b) = (Fake::at(0.5), Fake::at(0.5));
        let mut m = mixer([0.5, 0.5]);
        m.trims.insert("b.exe".into(), 0.5);
        m.faders[MEDIA].touched = Some(Instant::now());
        m.follow(windows_has(&[("a", "a.exe", &a, MEDIA), ("b", "b.exe", &b, MEDIA)]), &channels(), true);
        m.set_focus(Some(GAME), &channels());
        let r = m.change_channel(MEDIA, Some(10.0), &channels());
        assert_eq!((r.volume, r.live), (60, true), "the readout shows the real level");
        assert!(close(a.get(), 0.15) && close(b.get(), 0.075), "turned down: {} {}", a.get(), b.get());
        let r = m.change_channel(MEDIA, None, &channels());
        assert!(r.muted && a.is_muted() && b.is_muted(), "the mute key mutes the whole channel");
        assert!(close(a.get(), 0.15) && close(m.faders[MEDIA].volume, 0.6), "and leaves every level as it is");
        m.set_focus(None, &channels());
        assert!(close(a.get(), 0.6) && close(b.get(), 0.3), "real levels back: {} {}", a.get(), b.get());
    }

    #[test]
    fn names_and_meter_steps() {
        assert_eq!((pretty("discord.exe"), pretty("x"), pretty("")), ("Discord".into(), "X".into(), String::new()));
        assert_eq!((to_level(0.0), to_level(1.0)), (0, 255));
        assert_eq!(to_level(10f32.powf(-24.0 / 20.0)), 127, "-24 dBFS is half way on the -48..0 dB scale");
        assert_eq!(to_level(0.001), 0, "below -48 dBFS");
    }

    /// The desktop and taskbar (explorer.exe, pid 100) started Spotify (200) and a game (300).
    fn started_from_explorer() -> Snapshot {
        Snapshot::of(&[(100, "explorer.exe", 1), (200, "spotify.exe", 100), (300, "game.exe", 100)])
    }

    #[test]
    fn the_desktop_in_front_puts_no_app_in_the_focused_channel() {
        let ch = |name: &str, target| ChannelCfg { name: name.into(), color: [0; 3], target };
        let channels = vec![ch("Front", ChannelTarget::Focused), ch("Rest", ChannelTarget::Other)];
        let none = HashMap::new();
        let in_front = |fg: u32| Some((fg, started_from_explorer()));
        assert_eq!(assign(&channels, &none, "spotify.exe", 200, in_front(200).as_ref()), Some(0), "Spotify in front");
        assert_eq!(assign(&channels, &none, "game.exe", 300, in_front(200).as_ref()), Some(1));
        for exe_pid in [("spotify.exe", 200), ("game.exe", 300), ("explorer.exe", 100)] {
            let (exe, pid) = exe_pid;
            assert_eq!(assign(&channels, &none, exe, pid, in_front(100).as_ref()), Some(1), "desktop in front: {exe}");
        }
    }

    #[test]
    fn the_app_in_front_knob_turns_nothing_while_the_desktop_is_in_front() {
        let (spotify, game) = (Fake::at(0.3), Fake::at(0.9));
        let mut m = mixer([0.5, 0.5]);
        m.sessions = windows_has(&[("s", "spotify.exe", &spotify, MEDIA), ("g", "game.exe", &game, GAME)]);
        m.sessions[0].pid = 200;
        m.sessions[1].pid = 300;
        let r = m.change_in_front(&started_from_explorer(), Some(100), Some(10.0));
        assert!(!r.live && r.name == "Nothing in front", "{} {}", r.name, r.live);
        assert!(close(spotify.get(), 0.3) && close(game.get(), 0.9), "{} {}", spotify.get(), game.get());
        let r = m.change_in_front(&started_from_explorer(), Some(200), Some(10.0));
        assert!(r.live && r.name == "Spotify" && r.volume == 40);
        assert!(close(spotify.get(), 0.4) && close(game.get(), 0.9), "only the app in front");
    }

    #[test]
    fn programme_audio_counts_after_the_apps_own_volume_and_mute() {
        assert!(close(heard_peak(0.5, 0.5, false, false), 0.25));
        assert_eq!(heard_peak(0.5, 1.0, true, false), 0.0, "the app is muted");
        assert_eq!(heard_peak(0.5, 1.0, false, true), 0.0, "the default output device is muted");
        assert_eq!(heard_peak(0.009, 1.0, false, false), 0.0, "below about -40 dBFS");
        // Loud at the source but turned down to a whisper: the idle timers may run.
        let mut a = Audible::default();
        let t0 = Instant::now();
        let quiet = heard_peak(0.5, 0.01, false, false);
        assert!(!a.update(quiet, t0) && !a.update(quiet, t0 + Duration::from_secs(5)));
        assert!((20.0 * AUDIBLE.log10() - -40.0).abs() < 0.01, "AUDIBLE is -40 dBFS");
    }

    #[test]
    fn only_sustained_sound_counts_as_playing() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut a = Audible::default();
        assert!(!a.update(0.5, at(0)));
        assert!(!a.update(0.5, at(600)), "a ping");
        assert!(!a.update(0.0, at(700)));
        assert!(!a.update(0.5, at(800)), "starts over after the gap");
        assert!(!a.update(0.5, at(1700)));
        assert!(a.update(0.5, at(1800)), "a second of sound");
        assert!(a.update(0.0, at(2000)), "a pause between songs");
        assert!(a.update(0.5, at(3000)));
        assert!(a.update(0.005, at(3100)), "below -40 dBFS is quiet");
        assert!(!a.update(0.0, at(5600)), "quiet long enough");
    }
}
