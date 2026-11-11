//! End to end on real Windows audio: one channel holds two apps, and each app keeps a level of
//! its own. The app's own `audio` module moves the sliders; this test reads them back through
//! Core Audio after each step.
//!
//! Run it with:
//!
//!     cargo test --test audio_e2e -- --ignored
//!
//! It needs a default output device, and skips with a message when there is none. It builds
//! `examples/fake_player.rs` (a program that plays silence, so nothing is audible) and starts two
//! copies of it from a temporary folder, as `fake-spotify.exe` and `fake-browser.exe`. It
//! changes only the sessions of those two copies: never the master volume, the microphone or
//! the session of any other program, and it checks that the other sessions did not change.
//! The copies are stopped and the folder is deleted when the test ends, also when it fails.

use kb16_mixer::audio::Mixer;
use kb16_mixer::config::{ChannelCfg, ChannelTarget};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::core::Interface;

const SPOTIFY: &str = "fake-spotify.exe";
const BROWSER: &str = "fake-browser.exe";
const TEST: usize = 0;
const FOCUSED: usize = 1;

/// One session as Core Audio reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Slider {
    pid: u32,
    volume: f32,
    muted: bool,
}

/// Every app session on every active output device, by session instance id.
fn sessions() -> HashMap<String, Slider> {
    let mut out = HashMap::new();
    // SAFETY: read-only COM calls on objects created here; the id string is freed once, after
    // it was copied.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(devices) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
            return out;
        };
        let Ok(endpoints) = devices.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return out };
        for i in 0..endpoints.GetCount().unwrap_or(0) {
            let Ok(manager) = endpoints.Item(i).and_then(|d| d.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None))
            else {
                continue;
            };
            let Ok(list) = manager.GetSessionEnumerator() else { continue };
            for j in 0..list.GetCount().unwrap_or(0) {
                let Ok(control) = list.GetSession(j) else { continue };
                let (Ok(control2), Ok(volume)) =
                    (control.cast::<IAudioSessionControl2>(), control.cast::<ISimpleAudioVolume>())
                else {
                    continue;
                };
                let Ok(id) = control2.GetSessionInstanceIdentifier() else { continue };
                let key = id.to_string().unwrap_or_default();
                CoTaskMemFree(Some(id.0 as *const _));
                let slider = Slider {
                    pid: control2.GetProcessId().unwrap_or(0),
                    volume: volume.GetMasterVolume().unwrap_or(-1.0),
                    muted: volume.GetMute().map(|b| b.as_bool()).unwrap_or(false),
                };
                out.insert(key, slider);
            }
        }
    }
    out
}

fn has_default_output() -> bool {
    // SAFETY: read-only COM calls on an enumerator created here.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .and_then(|d| d.GetDefaultAudioEndpoint(eRender, eConsole))
            .is_ok()
    }
}

/// The two copies of the helper and their folder; both go away on drop, also after a failure.
struct Players {
    children: Vec<Child>,
    dir: PathBuf,
}

impl Players {
    fn start(helper: &Path) -> Players {
        let dir = std::env::temp_dir().join(format!("kb16-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp folder");
        let mut players = Players { children: Vec::new(), dir };
        for name in [SPOTIFY, BROWSER] {
            let exe = players.dir.join(name);
            std::fs::copy(helper, &exe).expect("copy the helper");
            // Its standard input stays open while the test runs: closing it stops the helper.
            let child = Command::new(&exe).stdin(Stdio::piped()).spawn().expect("start the helper");
            players.children.push(child);
        }
        players
    }

    fn pid(&self, name: &str) -> u32 {
        let i = if name == SPOTIFY { 0 } else { 1 };
        self.children[i].id()
    }
}

impl Drop for Players {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Builds the helper (a no-op when it is up to date) and returns its path.
fn build_helper() -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    // This test runs from target/<profile>/deps; examples go to target/<profile>/examples.
    let profile_dir = std::env::current_exe().expect("test exe").parent().and_then(Path::parent).unwrap().to_owned();
    let mut build = Command::new(cargo);
    build.args(["build", "--quiet", "--example", "fake_player", "--manifest-path"]).arg(&manifest);
    if profile_dir.ends_with("release") {
        build.arg("--release");
    }
    assert!(build.status().expect("run cargo").success(), "could not build examples/fake_player.rs");
    profile_dir.join("examples").join("fake_player.exe")
}

/// Waits until Windows lists a session for each pid, for 5 s at most.
fn wait_for_sessions(pids: &[u32]) {
    let start = Instant::now();
    while !pids.iter().all(|pid| sessions().values().any(|s| s.pid == *pid)) {
        assert!(start.elapsed() < Duration::from_secs(5), "the helpers did not get an audio session");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The volume and mute of every session of one pid (one, unless a device was added meanwhile).
fn read(pid: u32) -> Vec<(f32, bool)> {
    sessions().values().filter(|s| s.pid == pid).map(|s| (s.volume, s.muted)).collect()
}

#[track_caller]
fn assert_slider(players: &Players, name: &str, volume: f32, muted: bool, step: &str) {
    let seen = read(players.pid(name));
    assert!(!seen.is_empty(), "{step}: {name} has no session");
    for (v, m) in seen {
        assert!((v - volume).abs() < 0.002 && m == muted, "{step}: {name} is at {v} muted {m}, not {volume} {muted}");
    }
}

#[test]
#[ignore = "plays silence on the real default output device; run with --ignored"]
fn one_channel_holds_two_apps_with_their_own_levels() {
    if !has_default_output() {
        eprintln!("SKIPPED: this PC has no default audio output device");
        return;
    }
    let helper = build_helper();
    let started = Instant::now();
    let others_before = sessions();
    let players = Players::start(&helper);
    let ours = [players.pid(SPOTIFY), players.pid(BROWSER)];
    wait_for_sessions(&ours);

    let channel = |name: &str, apps: &[&str]| ChannelCfg {
        name: name.into(),
        color: [0; 3],
        target: ChannelTarget::Apps(apps.iter().map(|a| a.to_string()).collect()),
    };
    // No "other" channel: the sessions of every other program belong to no channel.
    let channels = vec![channel("Test", &[SPOTIFY, BROWSER]), channel("Nobody", &["kb16-e2e-nobody.exe"])];
    let mut mixer = Mixer::new().expect("Core Audio");
    mixer.reset(&channels, &HashMap::new());
    mixer.refresh(&channels, true);
    assert_eq!(mixer.apps_in(TEST), [BROWSER, SPOTIFY], "both helpers are in the Test channel");

    // (a) The channel level moves both apps.
    mixer.change_channel(TEST, Some(-100.0), &channels);
    mixer.change_channel(TEST, Some(60.0), &channels);
    assert_slider(&players, SPOTIFY, 0.6, false, "channel at 60 %");
    assert_slider(&players, BROWSER, 0.6, false, "channel at 60 %");

    // (b) A trim turns one app down; the other stays. The balance holds when the channel moves.
    mixer.change_app(TEST, SPOTIFY, -30.0, &channels);
    assert_slider(&players, SPOTIFY, 0.3, false, "spotify trimmed to half");
    assert_slider(&players, BROWSER, 0.6, false, "spotify trimmed to half");
    mixer.change_channel(TEST, Some(20.0), &channels);
    assert_slider(&players, SPOTIFY, 0.4, false, "channel at 80 %, trim kept");
    assert_slider(&players, BROWSER, 0.8, false, "channel at 80 %, trim kept");

    // (c) Mute mutes the whole channel and keeps every level.
    mixer.change_channel(TEST, None, &channels);
    assert_slider(&players, SPOTIFY, 0.4, true, "channel muted");
    assert_slider(&players, BROWSER, 0.8, true, "channel muted");
    mixer.change_channel(TEST, None, &channels);
    assert_slider(&players, SPOTIFY, 0.4, false, "channel unmuted");
    assert_slider(&players, BROWSER, 0.8, false, "channel unmuted");

    // (d) Focus on another channel turns this one down to a quarter, and puts it back.
    mixer.duck = 0.25;
    mixer.set_focus(Some(FOCUSED), &channels);
    assert_slider(&players, SPOTIFY, 0.1, false, "Focus on another channel");
    assert_slider(&players, BROWSER, 0.2, false, "Focus on another channel");
    mixer.set_focus(None, &channels);
    assert_slider(&players, SPOTIFY, 0.4, false, "Focus off");
    assert_slider(&players, BROWSER, 0.8, false, "Focus off");

    // No other session changed (sessions that came or went meanwhile are not compared).
    let others_after = sessions();
    let mut compared = 0;
    for (id, before) in &others_before {
        if let Some(after) = others_after.get(id).filter(|s| !ours.contains(&s.pid)) {
            assert_eq!(after, before, "another program's session changed");
            compared += 1;
        }
    }
    drop(players);
    println!("other sessions compared: {compared}; time: {:.1} s", started.elapsed().as_secs_f32());
    assert!(started.elapsed() < Duration::from_secs(30));
}
