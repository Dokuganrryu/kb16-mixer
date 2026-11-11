//! Enforces the promises of the app (CONTRIBUTING.md, "Code rules"): no hooks, no input
//! injection, no access to other processes and no network. The test reads the source, the
//! manifest and the lock file, and fails on a name that only such code needs. The names count
//! also in comments, so that the scan stays simple and has no exceptions.
//! It skips this file, because this file must spell the names.

use std::fs;
use std::path::{Path, PathBuf};

/// Windows API functions and Rust paths. A match at a name start is enough, so the A and W
/// variants (`SetWindowsHookExW`) are found too.
const FORBIDDEN_CODE: &[&str] = &[
    // Hooks
    "SetWindowsHookEx",
    "SetWinEventHook",
    // Input injection
    "SendInput",
    "keybd_event",
    "mouse_event",
    "Input::KeyboardAndMouse",
    // Access to other processes
    "OpenProcess",
    "ReadProcessMemory",
    "WriteProcessMemory",
    "CreateRemoteThread",
    "VirtualAllocEx",
    // Network
    "Win32::Networking",
    "WinHttp",
    "WinINet",
    "InternetOpen",
    "WSAStartup",
];

/// The network module of std and crates that exist to talk to the network. They match as whole
/// names only.
const FORBIDDEN_CRATES: &[&str] =
    &["std::net", "reqwest", "hyper", "ureq", "native-tls", "native_tls", "rustls", "openssl"];

/// Features of the `windows` crate that the app must never turn on.
const FORBIDDEN_FEATURES: &[&str] = &["Win32_UI_Input_KeyboardAndMouse", "Win32_Networking"];

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// True if `needle` is in `haystack` with no identifier character before it, and, when `whole`
/// is set, none after it.
fn contains_name(haystack: &str, needle: &str, whole: bool) -> bool {
    haystack.match_indices(needle).any(|(at, _)| {
        let before = haystack[..at].chars().next_back().is_none_or(|c| !is_ident(c) && c != '-');
        let after = haystack[at + needle.len()..].chars().next();
        before && (!whole || after.is_none_or(|c| !is_ident(c) && c != '-'))
    })
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn scan_code(name: &str, text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (n, line) in text.lines().enumerate() {
        for needle in FORBIDDEN_CODE {
            if contains_name(line, needle, false) {
                found.push(format!("{name}:{}: {needle}", n + 1));
            }
        }
        for needle in FORBIDDEN_CRATES {
            if contains_name(line, needle, true) {
                found.push(format!("{name}:{}: {needle}", n + 1));
            }
        }
    }
    found
}

fn scan_manifest(name: &str, text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (n, line) in text.lines().enumerate() {
        for needle in FORBIDDEN_FEATURES {
            if contains_name(line, needle, false) {
                found.push(format!("{name}:{}: feature {needle}", n + 1));
            }
        }
        for needle in FORBIDDEN_CRATES {
            if contains_name(line, needle, true) {
                found.push(format!("{name}:{}: crate {needle}", n + 1));
            }
        }
    }
    found
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

#[test]
fn app_keeps_its_promises() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src", "tests", "examples"] {
        rust_files(&root.join(dir), &mut files);
    }
    files.push(root.join("build.rs"));
    let this_file = root.join("tests").join("forbidden_apis.rs");
    files.retain(|f| f != &this_file && f.exists());
    assert!(files.len() > 10, "the scan found only {} Rust files; the folder layout changed", files.len());

    let mut found = Vec::new();
    for file in &files {
        let name = file.strip_prefix(root).unwrap_or(file).display().to_string();
        found.extend(scan_code(&name, &read(file)));
    }
    for manifest in ["Cargo.toml", "Cargo.lock"] {
        found.extend(scan_manifest(manifest, &read(&root.join(manifest))));
    }
    assert!(found.is_empty(), "the app must not use these (see CONTRIBUTING.md, Code rules):\n{}", found.join("\n"));
}

#[test]
fn the_scan_finds_what_it_must_find() {
    assert_eq!(scan_code("a.rs", "use std::net::TcpStream;").len(), 1);
    assert_eq!(scan_code("a.rs", "unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, ..) }").len(), 1);
    assert_eq!(scan_code("a.rs", "use windows::Win32::UI::Input::KeyboardAndMouse::SendInput;").len(), 2);
    assert_eq!(scan_code("a.rs", "let c = reqwest::blocking::Client::new();").len(), 1);
    assert_eq!(scan_manifest("Cargo.toml", r#"features = ["Win32_Foundation", "Win32_Networking_WinSock"]"#).len(), 1);
    assert_eq!(scan_manifest("Cargo.lock", r#"name = "tokio-rustls""#).len(), 0);
    assert_eq!(scan_manifest("Cargo.lock", r#"name = "rustls""#).len(), 1);
}

#[test]
fn the_scan_ignores_longer_names() {
    assert!(scan_code("a.rs", "// a hyperlink in the about box").is_empty());
    assert!(scan_code("a.rs", "fn my_SendInputs() {}").is_empty());
    assert!(scan_code("a.rs", "let hyper_threads = 2;").is_empty());
    assert!(scan_code("a.rs", "use std::network_name;").is_empty());
}
