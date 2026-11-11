//! kb16-mixer.exe. Everything is in the library (`lib.rs`); this only makes it a Windows app
//! without a console window.

#![windows_subsystem = "windows"]

fn main() {
    kb16_mixer::run();
}
