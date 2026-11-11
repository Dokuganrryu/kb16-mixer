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

pub fn foreground_pid() -> Option<u32> {
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
