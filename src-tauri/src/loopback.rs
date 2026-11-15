//! System audio capture ("what you hear") on Windows.
//!
//! FFmpeg has no loopback source for Windows: the `dshow` input only exposes
//! recording devices, so `audio="WASAPI loopback"` fails and the usual workaround
//! is an enabled "Stereo Mix" device or a virtual cable that the system output has
//! to be routed into. Both are opt-in system changes the app cannot depend on, and
//! when a virtual cable exists but nothing is routed into it the capture simply
//! records silence — which is what made "System" and "Both" produce files without
//! the game sound.
//!
//! This module captures the default render device directly through WASAPI in
//! loopback mode and hands the raw PCM to FFmpeg over a named pipe, so system audio
//! works out of the box with no device setup.

use anyhow::Result;

/// Every loopback pipe path starts with this prefix. `recorder` uses it to tell a
/// pipe from a DirectShow device name and pick the matching FFmpeg input options.
pub const PIPE_PREFIX: &str = r"\\.\pipe\specter-audio-";

/// Format of the samples written into the pipe. It is fixed instead of inheriting
/// the device mix format: the audio engine converts on its side, which keeps the
/// FFmpeg arguments constant and avoids any resampling code here.
pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;

pub fn is_pipe_path(path: &str) -> bool {
    path.starts_with(PIPE_PREFIX)
}

/// A running loopback capture. Dropping it stops the capture thread.
pub struct SystemAudioLoopback {
    path: String,
    #[cfg(windows)]
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(windows)]
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SystemAudioLoopback {
    /// Starts capturing the default playback device. The pipe exists by the time
    /// this returns, so FFmpeg can be started right after.
    #[cfg(windows)]
    pub fn start() -> Result<SystemAudioLoopback> {
        use anyhow::Context as _;
        use std::sync::atomic::AtomicBool;
        use std::sync::{mpsc, Arc};
        use std::time::Duration;

        let path = format!("{}{}", PIPE_PREFIX, unique_suffix());
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, signaled) = mpsc::channel();
        let thread_stop = Arc::clone(&stop);
        let thread_path = path.clone();
        let thread = std::thread::Builder::new()
            .name("specter-system-audio".to_string())
            .spawn(move || windows_impl::run(thread_path, thread_stop, ready))
            .context("Failed to start the system audio thread")?;

        match signaled.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(SystemAudioLoopback {
                path,
                stop,
                thread: Some(thread),
            }),
            Ok(Err(err)) => {
                let _ = thread.join();
                Err(err)
            }
            Err(_) => {
                stop.store(true, std::sync::atomic::Ordering::SeqCst);
                windows_impl::unblock(&path);
                let _ = thread.join();
                anyhow::bail!("Timed out while starting system audio capture")
            }
        }
    }

    #[cfg(not(windows))]
    pub fn start() -> Result<SystemAudioLoopback> {
        anyhow::bail!("System audio capture is only implemented on Windows")
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// FFmpeg input arguments for the raw PCM carried by the pipe.
    pub fn input_args(&self) -> Vec<String> {
        vec![
            "-f".to_string(),
            "s16le".to_string(),
            "-ar".to_string(),
            SAMPLE_RATE.to_string(),
            "-ac".to_string(),
            CHANNELS.to_string(),
            "-i".to_string(),
            self.path.clone(),
        ]
    }
}

impl Drop for SystemAudioLoopback {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            windows_impl::unblock(&self.path);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    use anyhow::{bail, Context};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX, WAVE_FORMAT_PCM,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE, FILE_SHARE_MODE,
        OPEN_EXISTING, PIPE_ACCESS_OUTBOUND,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    /// Room for about ten seconds of stereo PCM, so a momentarily busy FFmpeg never
    /// stalls the capture thread.
    const PIPE_BUFFER: u32 = SAMPLE_RATE * CHANNELS as u32 * 2 * 10;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn create_pipe(path: &str) -> Result<HANDLE> {
        let name = wide(path);
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_OUTBOUND,
                PIPE_TYPE_BYTE | PIPE_WAIT,
                1,
                PIPE_BUFFER,
                PIPE_BUFFER,
                0,
                None,
            )
        };
        if handle.is_invalid() {
            bail!("Failed to create the system audio pipe");
        }
        Ok(handle)
    }

    /// Opens the pipe from the client side. `Drop` uses it to release a capture
    /// thread that is still waiting for FFmpeg to connect.
    pub(super) fn unblock(path: &str) {
        let name = wide(path);
        unsafe {
            if let Ok(handle) = CreateFileW(
                PCWSTR(name.as_ptr()),
                FILE_GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                HANDLE(std::ptr::null_mut()),
            ) {
                let _ = CloseHandle(handle);
            }
        }
    }

    fn write_all(pipe: HANDLE, mut data: &[u8]) -> bool {
        while !data.is_empty() {
            let mut written = 0u32;
            let result = unsafe {
                WriteFile(
                    pipe,
                    Some(&data[..data.len().min(u32::MAX as usize)]),
                    Some(&mut written),
                    None,
                )
            };
            if result.is_err() || written == 0 {
                return false;
            }
            data = &data[written as usize..];
        }
        true
    }

    pub(super) fn run(path: String, stop: Arc<AtomicBool>, ready: mpsc::Sender<Result<()>>) {
        // The pipe has to exist before FFmpeg is started, so it is created here and
        // reported back immediately.
        let pipe = match create_pipe(&path) {
            Ok(pipe) => pipe,
            Err(err) => {
                let _ = ready.send(Err(err));
                return;
            }
        };
        if ready.send(Ok(())).is_err() {
            unsafe {
                let _ = CloseHandle(pipe);
            }
            return;
        }

        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let capture = match open_capture() {
            Ok(capture) => capture,
            Err(_) => {
                unsafe {
                    let _ = CloseHandle(pipe);
                }
                return;
            }
        };

        // Blocks until FFmpeg opens the input. `Drop` connects a dummy client to
        // wake the thread up when the recording is cancelled before that happens.
        unsafe {
            let _ = ConnectNamedPipe(pipe, None);
        }

        if !stop.load(Ordering::SeqCst) {
            if capture.start().is_ok() {
                pump(&capture, pipe, &stop);
                let _ = capture.stop();
            }
        }

        unsafe {
            let _ = CloseHandle(pipe);
            CoUninitialize();
        }
    }

    struct Capture {
        client: IAudioClient,
        source: IAudioCaptureClient,
    }

    impl Capture {
        fn start(&self) -> Result<()> {
            unsafe { self.client.Start() }.map_err(Into::into)
        }

        fn stop(&self) -> Result<()> {
            unsafe { self.client.Stop() }.map_err(Into::into)
        }
    }

    fn open_capture() -> Result<Capture> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .context("Failed to open the audio device enumerator")?;
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
            .context("No playback device is available for system audio capture")?;
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
            .context("Failed to activate the playback device")?;

        let block_align = CHANNELS * 2;
        let format = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: CHANNELS,
            nSamplesPerSec: SAMPLE_RATE,
            nAvgBytesPerSec: SAMPLE_RATE * block_align as u32,
            nBlockAlign: block_align,
            wBitsPerSample: 16,
            cbSize: 0,
        };

        // Loopback capture of the default render device: everything the system
        // plays is returned here, no Stereo Mix or virtual cable required.
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                200 * 10_000,
                0,
                &format,
                None,
            )
        }
        .context("Failed to open the playback device in loopback mode")?;

        let source: IAudioCaptureClient = unsafe { client.GetService() }
            .context("Failed to obtain the loopback capture interface")?;

        Ok(Capture { client, source })
    }

    fn pump(capture: &Capture, pipe: HANDLE, stop: &Arc<AtomicBool>) {
        let bytes_per_second = SAMPLE_RATE as u64 * CHANNELS as u64 * 2;
        // The audio track must never stall the muxer, so the pipe is paced by the
        // wall clock: loopback delivers nothing at all while the endpoint is idle,
        // and FFmpeg would then sit waiting for audio while the desktop frames pile
        // up in `ddagrab` and are lost.
        const PAD_MAX_SECONDS: u64 = 1;
        let started = std::time::Instant::now();
        let mut written: u64 = 0;

        'outer: while !stop.load(Ordering::SeqCst) {
            loop {
                let available = match unsafe { capture.source.GetNextPacketSize() } {
                    Ok(available) => available,
                    Err(_) => break 'outer,
                };
                if available == 0 {
                    break;
                }

                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                if unsafe {
                    capture
                        .source
                        .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                }
                .is_err()
                {
                    break 'outer;
                }

                let bytes = frames as usize * CHANNELS as usize * 2;
                let is_silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null();
                let ok = if is_silent {
                    write_all(pipe, &vec![0u8; bytes])
                } else {
                    write_all(pipe, unsafe { std::slice::from_raw_parts(data, bytes) })
                };
                let _ = unsafe { capture.source.ReleaseBuffer(frames) };

                if !ok {
                    break 'outer;
                }
                written += bytes as u64;
            }

            let expected = (started.elapsed().as_secs_f64() * bytes_per_second as f64) as u64;
            let deficit = expected.saturating_sub(written);
            if deficit >= bytes_per_second / 100 {
                // Always frame-aligned: FFmpeg rejects a PCM stream that ends
                // mid-frame.
                let align = CHANNELS as u64 * 2;
                let pad = (deficit.min(bytes_per_second * PAD_MAX_SECONDS) / align) * align;
                if pad > 0 && !write_all(pipe, &vec![0u8; pad as usize]) {
                    break;
                }
                written += pad;
            }

            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(windows)]
fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}
