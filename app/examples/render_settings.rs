//! Draws the settings window and its dialogs into BMP files without ever showing them, and
//! measures what the window costs. For the design checks and the pictures in docs/images.
//!
//! The window reads and writes the config of KB16_MIXER_DIR, so that must name a throwaway
//! folder; the program refuses to run without it. Nothing appears on the screen: every window
//! is made hidden and painted through WM_PRINT (see `kb16_mixer::render_hidden`).
//!
//!   KB16_MIXER_DIR=<temp folder> cargo run --example render_settings -- <out folder> [what]
//!
//! `what` is `all` (the default: every tab and dialog, dark and light, at 96, 120, 144 and
//! 192 dpi), `docs` (the three dark tabs at 96 dpi), `worst` (the worst-case texts) or
//! `measure` (the budget figures: time to the first picture, working set, GDI objects).
//!
//! Test and example programs get no embedded manifest, so this one writes
//! `render_settings.exe.manifest` beside itself (modern controls, per-monitor scaling) and asks
//! to be run again once, if it was missing.

use kb16_mixer::{Look, Options, Picture, Preview, Tab, render_hidden};
use std::path::{Path, PathBuf};
use windows::Win32::System::Threading::{
    GR_GDIOBJECTS, GR_USEROBJECTS, GetCurrentProcess, GetGuiResources, GetProcessHandleCount, GetProcessTimes,
};

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency><dependentAssembly><assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls"
    version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/></dependentAssembly></dependency>
  <application xmlns="urn:schemas-microsoft-com:asm.v3"><windowsSettings>
    <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
  </windowsSettings></application>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1"><application>
    <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/></application></compatibility>
</assembly>
"#;

/// PROCESS_MEMORY_COUNTERS, for the working set.
#[repr(C)]
#[derive(Default)]
struct MemoryCounters {
    cb: u32,
    page_faults: u32,
    peak_working_set: usize,
    working_set: usize,
    quota: [usize; 4],
    pagefile: usize,
    peak_pagefile: usize,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn K32GetProcessMemoryInfo(process: windows::Win32::Foundation::HANDLE, c: *mut MemoryCounters, cb: u32) -> i32;
}

fn working_set() -> usize {
    let mut c = MemoryCounters { cb: size_of::<MemoryCounters>() as u32, ..Default::default() };
    // SAFETY: the struct carries its own size; the handle is this process's pseudo-handle.
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    c.working_set
}

fn objects() -> (u32, u32, u32) {
    let mut handles = 0;
    // SAFETY: plain queries about this process.
    unsafe {
        let p = GetCurrentProcess();
        let _ = GetProcessHandleCount(p, &mut handles);
        (GetGuiResources(p, GR_GDIOBJECTS), GetGuiResources(p, GR_USEROBJECTS), handles)
    }
}

/// CPU time of this process so far, in milliseconds.
fn cpu_ms() -> f64 {
    let (mut a, mut b, mut k, mut u) = Default::default();
    // SAFETY: four FILETIMEs written by the call.
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut k, &mut u);
    }
    let ms = |t: windows::Win32::Foundation::FILETIME| {
        ((t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64) as f64 / 1e4
    };
    ms(k) + ms(u)
}

/// Writes a 24-bit BMP.
fn write_bmp(path: &Path, p: &Picture) -> std::io::Result<()> {
    let row = (p.width as usize * 3).div_ceil(4) * 4;
    let size = 54 + row * p.height as usize;
    let mut b = Vec::with_capacity(size);
    b.extend(b"BM");
    for v in [size as u32, 0, 54, 40, p.width as u32, p.height as u32] {
        b.extend(v.to_le_bytes());
    }
    b.extend(1u16.to_le_bytes());
    b.extend(24u16.to_le_bytes());
    b.extend([0u8; 24]);
    for y in (0..p.height as usize).rev() {
        let start = b.len();
        for x in 0..p.width as usize {
            let c = p.pixels[y * p.width as usize + x];
            b.extend([c as u8, (c >> 8) as u8, (c >> 16) as u8]);
        }
        b.resize(start + row, 0);
    }
    std::fs::write(path, b)
}

fn name(what: &Preview) -> &'static str {
    match what {
        Preview::Tab(Tab::Keys) => "keys",
        Preview::Tab(Tab::Channels) => "channels",
        Preview::Tab(Tab::Pad) => "pad",
        Preview::Shortcut(_) => "shortcut",
        Preview::Open(_) => "open",
        Preview::AddProgram => "program",
    }
}

fn render(out: &Path, what: Preview, o: &Options, suffix: &str) {
    let look = match o.look {
        Some(Look::Light) => "light",
        Some(Look::Contrast) => "contrast",
        _ => "dark",
    };
    let file = out.join(format!("{}-{look}-{}{suffix}.bmp", name(&what), o.dpi));
    match render_hidden(what, o) {
        Ok(p) => match write_bmp(&file, &p) {
            Ok(()) => println!("{}  {}x{}  {:.1} ms", file.display(), p.width, p.height, p.took.as_secs_f64() * 1e3),
            Err(e) => println!("{}: {e}", file.display()),
        },
        Err(e) => println!("{}: {e}", file.display()),
    }
}

/// The default config with the longest real texts: a shortcut, a long "open:" target, the
/// longest app name first in a list, and a pad name of 7 letters.
fn worst_config() -> String {
    kb16_mixer::config::DEFAULT
        .replace(
            r#"row2 = ["move", "win+shift+s", "F13",   "F14"]"#,
            r#"row2 = ["move", "win+shift+s", "F13",   "open:C:\\Tools\\a-very-long-program-name.exe"]"#,
        )
        .replace(r#"apps = ["Discord.exe", "#, r#"apps = ["DiscordCanary.exe", "Discord.exe", "#)
        .replace(r#"name = "KB16""#, r#"name = "DESK PC""#)
}

const PROBLEM: &str = "config.toml has a problem, so the settings in force stay as they are and nothing is saved until it is fixed (Open config file on the Pad tab): TOML parse error at line 21, column 14: invalid float, expected a digit after the decimal point in the brightness entry";

fn measure() {
    thread_local! {
        static IDLE: std::cell::Cell<(f64, usize, u32)> = const { std::cell::Cell::new((0.0, 0, 0)) };
    }
    /// With the window open: 1 s of message pumping, its CPU time and timer messages counted.
    fn idle() {
        use windows::Win32::UI::WindowsAndMessaging::*;
        let (cpu, start) = (cpu_ms(), std::time::Instant::now());
        let mut timers = 0;
        let mut msg = MSG::default();
        while start.elapsed().as_millis() < 1000 {
            // SAFETY: a plain message loop of this thread.
            unsafe {
                let _ = MsgWaitForMultipleObjects(None, false, 50, QS_ALLINPUT);
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    timers += (msg.message == WM_TIMER) as u32;
                    DispatchMessageW(&msg);
                }
            }
        }
        IDLE.set((cpu_ms() - cpu, working_set(), timers));
    }
    let o = Options { dpi: 96, look: Some(Look::Dark), ..Default::default() };
    let before = working_set();
    let mut first = Vec::new();
    for tab in [Tab::Keys, Tab::Channels, Tab::Pad] {
        let p = render_hidden(Preview::Tab(tab), &Options { while_open: Some(idle), ..o });
        let (cpu, ws, timers) = IDLE.get();
        if let Ok(p) = p {
            first.push(p.took.as_secs_f64() * 1e3);
            println!(
                "{tab:?}: first picture {:.1} ms; open and idle 1 s: {cpu:.1} ms CPU, {timers} timer messages, working set +{} KB",
                p.took.as_secs_f64() * 1e3,
                (ws as i64 - before as i64) / 1024
            );
        }
    }
    let start = objects();
    for i in 0..20 {
        let tab = [Tab::Keys, Tab::Channels, Tab::Pad][i % 3];
        let _ = render_hidden(Preview::Tab(tab), &o);
    }
    let _ = render_hidden(Preview::Shortcut("win+shift+s"), &o);
    let _ = render_hidden(Preview::Open("C:\\Tools\\obs64.exe"), &o);
    let end = objects();
    println!("before 20 open/close cycles: GDI {}, USER {}, handles {}", start.0, start.1, start.2);
    println!("after 20 open/close cycles:  GDI {}, USER {}, handles {}", end.0, end.1, end.2);
    println!("working set now: {} KB (before the first window: {} KB)", working_set() / 1024, before / 1024);
}

fn main() {
    let Some(dir) = std::env::var_os("KB16_MIXER_DIR").filter(|d| !d.is_empty()).map(PathBuf::from) else {
        eprintln!("Set KB16_MIXER_DIR to a throwaway folder first: the window reads and writes its config.toml.");
        std::process::exit(2);
    };
    let exe = std::env::current_exe().unwrap_or_default();
    let manifest = PathBuf::from(format!("{}.manifest", exe.display()));
    if !manifest.exists() {
        let _ = std::fs::write(&manifest, MANIFEST);
        eprintln!("Wrote {}; run this again so that Windows reads it.", manifest.display());
        std::process::exit(3);
    }
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let what = args.next().unwrap_or_else(|| "all".into());
    let _ = std::fs::create_dir_all(&out);
    let config = dir.join("config.toml");
    if !config.exists() {
        let _ = std::fs::write(&config, kb16_mixer::config::DEFAULT);
    }
    let tabs = [Preview::Tab(Tab::Keys), Preview::Tab(Tab::Channels), Preview::Tab(Tab::Pad)];
    let dialogs = [Preview::Shortcut("win+shift+s"), Preview::Open(""), Preview::AddProgram];
    match what.as_str() {
        "measure" => measure(),
        "docs" => {
            for t in tabs {
                render(&out, t, &Options { dpi: 96, look: Some(Look::Dark), ..Default::default() }, "");
            }
        }
        "worst" => {
            let _ = std::fs::write(&config, worst_config());
            for dpi in [96, 144] {
                let o = Options { dpi, look: Some(Look::Dark), ..Default::default() };
                for t in tabs {
                    render(&out, t, &o, "-worst");
                }
                render(&out, tabs[0], &Options { status: Some(PROBLEM), ..o }, "-problem");
                render(&out, tabs[0], &Options { focus: Some(105), ..o }, "-focus");
                render(&out, tabs[2], &Options { focus: Some(307), ..o }, "-focus");
                render(&out, Preview::Open("C:\\Tools\\a-very-long-program-name.exe"), &o, "-worst");
            }
            let _ = std::fs::write(&config, kb16_mixer::config::DEFAULT);
        }
        _ => {
            for look in [Look::Dark, Look::Light, Look::Contrast] {
                for dpi in [96, 120, 144, 192] {
                    if look == Look::Contrast && dpi != 96 {
                        continue;
                    }
                    let o = Options { dpi, look: Some(look), ..Default::default() };
                    for w in tabs.iter().chain(&dialogs) {
                        render(&out, *w, &o, "");
                    }
                }
            }
        }
    }
}
