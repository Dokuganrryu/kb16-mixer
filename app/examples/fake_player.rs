//! Plays silence on the default output device, so that Windows lists an audio session for this
//! program. Only `tests/audio_e2e.rs` uses it; it is not part of the app or of a release.
//! It stops when its standard input closes (the test ends or is stopped), or after 60 s.

use std::io::Read;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

/// The shared-mode buffer, in 100 ns units.
const BUFFER: i64 = 2_000_000;
const LIFETIME: Duration = Duration::from_secs(60);

fn main() -> windows::core::Result<()> {
    let (closed, stdin_closed) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
        let _ = closed.send(());
    });
    // SAFETY: COM calls on objects this function creates, in the order WASAPI asks for. The mix
    // format is freed once, after `Initialize` has copied it; each buffer is released with the
    // frame count it was taken with, and SILENT means its bytes are never read.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let client: IAudioClient = devices.GetDefaultAudioEndpoint(eRender, eConsole)?.Activate(CLSCTX_ALL, None)?;
        let format = client.GetMixFormat()?;
        let initialized = client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, BUFFER, 0, format, None);
        CoTaskMemFree(Some(format as *const _));
        initialized?;
        let frames = client.GetBufferSize()?;
        let render: IAudioRenderClient = client.GetService()?;
        client.Start()?;
        let start = Instant::now();
        while start.elapsed() < LIFETIME
            && stdin_closed.recv_timeout(Duration::from_millis(50)) == Err(RecvTimeoutError::Timeout)
        {
            let free = frames - client.GetCurrentPadding()?;
            if free > 0 {
                render.GetBuffer(free)?;
                render.ReleaseBuffer(free, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
            }
        }
        client.Stop()
    }
}
