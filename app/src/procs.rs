//! Process names without opening any process handle (Toolhelp snapshot only),
//! so protected game processes are never touched.

use std::collections::HashMap;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

pub struct Snapshot {
    procs: HashMap<u32, (String, u32)>, // pid -> (exe lowercase, parent pid)
}

impl Snapshot {
    pub fn take() -> Snapshot {
        let mut procs = HashMap::new();
        // SAFETY: `e` is a live PROCESSENTRY32W with its size set, and the snapshot handle is
        // closed once. Only process names are read; no process is opened.
        unsafe {
            if let Ok(h) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
                let mut ok = Process32FirstW(h, &mut e).is_ok();
                while ok {
                    let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                    let exe = String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase();
                    procs.insert(e.th32ProcessID, (exe, e.th32ParentProcessID));
                    ok = Process32NextW(h, &mut e).is_ok();
                }
                let _ = CloseHandle(h);
            }
        }
        Snapshot { procs }
    }

    /// A made-up process list, (pid, exe, parent pid), for tests of the rules that read one.
    #[cfg(test)]
    pub fn of(list: &[(u32, &str, u32)]) -> Snapshot {
        Snapshot { procs: list.iter().map(|&(pid, exe, parent)| (pid, (exe.to_string(), parent))).collect() }
    }

    pub fn exe(&self, pid: u32) -> Option<&str> {
        self.procs.get(&pid).map(|(e, _)| e.as_str())
    }

    /// True if `pid` is `ancestor` or one of its descendants.
    pub fn descends_from(&self, mut pid: u32, ancestor: u32) -> bool {
        for _ in 0..16 {
            if pid == ancestor {
                return true;
            }
            match self.procs.get(&pid) {
                Some((_, parent)) if *parent != 0 && *parent != pid => pid = *parent,
                _ => return false,
            }
        }
        false
    }
}

/// The exe of the app in front (process `fg`), or None when that is no app: a process gone from
/// the list, or explorer.exe (the desktop, the taskbar, a folder window). Every app started from
/// the desktop descends from explorer.exe, so taking it as "in front" would claim them all.
pub fn app_in_front(snap: &Snapshot, fg: u32) -> Option<&str> {
    snap.exe(fg).filter(|exe| !exe.is_empty() && *exe != "explorer.exe")
}

pub fn foreground_pid() -> Option<u32> {
    // SAFETY: both calls take plain values; `pid` is a live u32 for the call.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}
