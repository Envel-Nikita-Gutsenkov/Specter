use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::ring_buffer::RingBuffer;
use crate::settings::Settings;
use crate::storage::StorageManager;
use crate::supervisor::ProcessSupervisor;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct BufferStats {
    pub used_mb: f64,
    pub limit_mb: f64,
    pub duration_seconds: f64,
    pub is_recording_to_disk: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Monitor {
    pub id: String,
    pub name: String,
    pub resolution: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WindowInfo {
    pub pid: u32,
    pub title: String,
    pub exe: String,
}

pub struct DiskRecordingState {
    pub writer: BufWriter<File>,
    pub temp_path: PathBuf,
    pub final_path: PathBuf,
    pub start_timestamp: f64,
    pub initial_duration: f64,
    pub total_bytes: usize,
}

pub struct CaptureEngine {
    buffer: Arc<Mutex<Option<RingBuffer>>>,
    settings: Arc<Mutex<Option<Settings>>>,
    reader_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    worker_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    supervisor: Arc<Mutex<Option<ProcessSupervisor>>>,
    disk_recording: Arc<Mutex<Option<DiskRecordingState>>>,
}

impl CaptureEngine {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(None)),
            settings: Arc::new(Mutex::new(None)),
            reader_handle: Arc::new(Mutex::new(None)),
            worker_handle: Arc::new(Mutex::new(None)),
            supervisor: Arc::new(Mutex::new(None)),
            disk_recording: Arc::new(Mutex::new(None)),
        }
    }

    pub fn start(&self, settings: Settings) -> anyhow::Result<()> {
        let (supervisor, stdout) = ProcessSupervisor::spawn(&settings)?;

        let limit_mb = settings.ram_limit_mb;
        let max_minutes = settings.buffer_minutes;
        let buf_arc = Arc::clone(&self.buffer);
        let disk_rec_arc = Arc::clone(&self.disk_recording);

        {
            let mut b = buf_arc.lock().unwrap();
            *b = Some(RingBuffer::new(limit_mb, max_minutes));
        }
        {
            let mut s = self.settings.lock().unwrap();
            *s = Some(settings);
        }
        {
            let mut sup = self.supervisor.lock().unwrap();
            *sup = Some(supervisor);
        }

        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(1024);

        let reader_thread = std::thread::Builder::new()
            .name("specter-pipe-drainer".to_string())
            .spawn(move || {
                let mut reader = stdout;
                let mut tmp = vec![0u8; 65536];
                loop {
                    match reader.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(tmp[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .context("Failed to spawn capture pipe reader thread")?;

        let worker_thread = std::thread::Builder::new()
            .name("specter-buffer-worker".to_string())
            .spawn(move || {
                while let Ok(chunk) = rx.recv() {
                    let ts = Self::current_timestamp();
                    let mut rec_guard = disk_rec_arc.lock().unwrap();
                    if let Some(rec) = rec_guard.as_mut() {
                        let _ = rec.writer.write_all(&chunk);
                        rec.total_bytes += chunk.len();
                    } else {
                        drop(rec_guard);
                        let mut guard = buf_arc.lock().unwrap();
                        if let Some(inner) = guard.as_mut() {
                            inner.push(chunk, ts);
                        }
                    }
                }
            })
            .context("Failed to spawn buffer worker thread")?;

        {
            let mut rh = self.reader_handle.lock().unwrap();
            *rh = Some(reader_thread);
        }
        {
            let mut wh = self.worker_handle.lock().unwrap();
            *wh = Some(worker_thread);
        }

        Ok(())
    }

    pub fn stop(&self) -> anyhow::Result<()> {
        if self.is_recording_to_disk() {
            let _ = self.stop_disk_recording();
        }
        if let Some(mut sup) = self.supervisor.lock().unwrap().take() {
            sup.terminate();
        }
        if let Some(handle) = self.reader_handle.lock().unwrap().take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.worker_handle.lock().unwrap().take() {
            let _ = handle.join();
        }
        {
            let mut b = self.buffer.lock().unwrap();
            if let Some(inner) = b.as_mut() {
                inner.clear();
            }
            *b = None;
        }
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.buffer.lock().unwrap().is_some()
    }

    pub fn is_recording_to_disk(&self) -> bool {
        self.disk_recording.lock().unwrap().is_some()
    }

    pub fn start_disk_recording(&self) -> anyhow::Result<String> {
        if self.is_recording_to_disk() {
            anyhow::bail!("Already recording to disk");
        }

        let output_folder = {
            let s_guard = self.settings.lock().unwrap();
            let s = s_guard.as_ref().context("No settings configured")?;
            s.output_folder.clone()
        };

        let folder = StorageManager::resolve_output_folder(&output_folder);
        std::fs::create_dir_all(&folder).context("Failed to create destination directory")?;

        let timestamp = StorageManager::format_timestamp();
        let final_filename = format!("Specter_{}.mp4", timestamp);
        let final_path = folder.join(&final_filename);
        let temp_path = folder.join(format!(".Specter_{}.mp4.tmp", timestamp));

        let file = File::create(&temp_path).context("Failed to create temporary recording file")?;
        let mut writer = BufWriter::new(file);

        let (initial_duration, total_bytes) = {
            let mut buf_guard = self.buffer.lock().unwrap();
            let inner = buf_guard.as_mut().context("No active replay buffer")?;

            let dur = match (inner.chunks.front(), inner.chunks.back()) {
                (Some(first), Some(last)) => (last.timestamp - first.timestamp).max(0.0),
                _ => 0.0,
            };

            inner.save_to_writer(&mut writer).context("Failed to write buffer pre-roll to file")?;
            writer.flush().context("Failed to flush writer")?;

            let bytes = inner.total_bytes;

            inner.chunks.clear();
            inner.chunks.shrink_to_fit();
            inner.total_bytes = inner.init_header.len();

            (dur, bytes)
        };

        let state = DiskRecordingState {
            writer,
            temp_path,
            final_path: final_path.clone(),
            start_timestamp: Self::current_timestamp(),
            initial_duration,
            total_bytes,
        };

        *self.disk_recording.lock().unwrap() = Some(state);

        Ok(final_path.to_string_lossy().to_string())
    }

    pub fn stop_disk_recording(&self) -> anyhow::Result<String> {
        let rec_opt = self.disk_recording.lock().unwrap().take();
        let mut rec = rec_opt.context("No active disk recording to stop")?;

        let _ = rec.writer.flush();
        drop(rec.writer);

        StorageManager::finalize_temp_file(&rec.temp_path, &rec.final_path)?;

        Ok(rec.final_path.to_string_lossy().to_string())
    }

    pub fn save_clip(&self, continue_recording: bool) -> anyhow::Result<String> {
        if continue_recording {
            if self.is_recording_to_disk() {
                let final_path = self.stop_disk_recording()?;
                Ok(final_path)
            } else {
                let path = self.start_disk_recording()?;
                Ok(format!("RECORDING_STARTED:{}", path))
            }
        } else {
            if self.is_recording_to_disk() {
                let final_path = self.stop_disk_recording()?;
                self.stop()?;
                Ok(final_path)
            } else {
                let output_folder = {
                    let settings = self.settings.lock().unwrap();
                    let s = settings.as_ref().context("No settings configured")?;
                    s.output_folder.clone()
                };

                let final_path = {
                    let buf = self.buffer.lock().unwrap();
                    let inner = buf.as_ref().context("No active replay buffer")?;
                    StorageManager::save_atomic(inner, &output_folder)?
                };

                self.stop()?;
                Ok(final_path)
            }
        }
    }

    pub fn get_stats(&self) -> BufferStats {
        let rec_guard = self.disk_recording.lock().unwrap();
        if let Some(rec) = rec_guard.as_ref() {
            let now = Self::current_timestamp();
            let live_duration = (now - rec.start_timestamp).max(0.0);
            let duration_seconds = rec.initial_duration + live_duration;
            let used_mb = rec.total_bytes as f64 / (1024.0 * 1024.0);
            let settings = self.settings.lock().unwrap();
            let limit_mb = settings.as_ref().map(|s| s.ram_limit_mb).unwrap_or(500);
            return BufferStats {
                used_mb,
                limit_mb: limit_mb as f64,
                duration_seconds,
                is_recording_to_disk: true,
            };
        }
        drop(rec_guard);

        let buf = self.buffer.lock().unwrap();
        if let Some(inner) = buf.as_ref() {
            let settings = self.settings.lock().unwrap();
            let limit_mb = settings.as_ref().map(|s| s.ram_limit_mb).unwrap_or(500);
            let used_mb = inner.total_bytes as f64 / (1024.0 * 1024.0);
            let duration_seconds = match (inner.chunks.front(), inner.chunks.back()) {
                (Some(first), Some(last)) => (last.timestamp - first.timestamp).max(0.0),
                _ => 0.0,
            };
            BufferStats {
                used_mb,
                limit_mb: limit_mb as f64,
                duration_seconds,
                is_recording_to_disk: false,
            }
        } else {
            BufferStats::default()
        }
    }

    fn current_timestamp() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs_f64()
    }
}

#[derive(Clone, Debug)]
pub struct MonitorRect {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg(windows)]
pub fn enumerate_monitors_with_rect() -> Vec<MonitorRect> {
    use std::mem;

    type BOOL = i32;
    type HMONITOR = *mut std::ffi::c_void;
    type HDC = *mut std::ffi::c_void;
    type LPRECT = *mut RECT;
    type LPARAM = isize;

    #[repr(C)]
    struct RECT {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct MONITORINFOEXW {
        cb_size: u32,
        rc_monitor: RECT,
        rc_work: RECT,
        dw_flags: u32,
        sz_device: [u16; 32],
    }

    #[link(name = "user32")]
    extern "system" {
        fn EnumDisplayMonitors(
            hdc: HDC,
            lprcClip: *const RECT,
            lpfnEnum: Option<unsafe extern "system" fn(HMONITOR, HDC, LPRECT, LPARAM) -> BOOL>,
            dwData: LPARAM,
        ) -> BOOL;
        fn GetMonitorInfoW(hMonitor: HMONITOR, lpmi: *mut MONITORINFOEXW) -> BOOL;
    }

    let mut rects: Vec<MonitorRect> = Vec::new();

    unsafe extern "system" fn callback(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: LPRECT,
        lparam: LPARAM,
    ) -> BOOL {
        let list = &mut *(lparam as *mut Vec<MonitorRect>);
        let mut info: MONITORINFOEXW = mem::zeroed();
        info.cb_size = mem::size_of::<MONITORINFOEXW>() as u32;

        if GetMonitorInfoW(hmon, &mut info) != 0 {
            let width = info.rc_monitor.right - info.rc_monitor.left;
            let height = info.rc_monitor.bottom - info.rc_monitor.top;
            list.push(MonitorRect {
                left: info.rc_monitor.left,
                top: info.rc_monitor.top,
                width,
                height,
            });
        }
        1
    }

    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(callback),
            &mut rects as *mut _ as LPARAM,
        );
    }

    rects
}

#[cfg(not(windows))]
pub fn enumerate_monitors_with_rect() -> Vec<MonitorRect> {
    vec![]
}

#[cfg(windows)]
pub fn enumerate_monitors() -> Vec<Monitor> {
    let rects = enumerate_monitors_with_rect();
    if rects.is_empty() {
        return vec![Monitor {
            id: "screen:0".to_string(),
            name: "Display 1 (Primary)".to_string(),
            resolution: "1920x1080".to_string(),
        }];
    }

    rects
        .into_iter()
        .enumerate()
        .map(|(idx, r)| Monitor {
            id: format!("screen:{}", idx),
            name: if idx == 0 {
                format!("Display {} (Primary)", idx + 1)
            } else {
                format!("Display {}", idx + 1)
            },
            resolution: format!("{}x{}", r.width, r.height),
        })
        .collect()
}

#[cfg(not(windows))]
pub fn enumerate_monitors() -> Vec<Monitor> {
    vec![Monitor {
        id: "screen:0".to_string(),
        name: "Display 1 (Primary)".to_string(),
        resolution: "1920x1080".to_string(),
    }]
}

#[cfg(windows)]
pub fn enumerate_windows() -> Vec<WindowInfo> {
    type HWND = *mut std::ffi::c_void;
    type BOOL = i32;
    type LPARAM = isize;
    type DWORD = u32;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(lpEnumFunc: unsafe extern "system" fn(HWND, LPARAM) -> BOOL, lParam: LPARAM) -> BOOL;
        fn IsWindowVisible(hWnd: HWND) -> BOOL;
        fn GetWindowTextLengthW(hWnd: HWND) -> i32;
        fn GetWindowTextW(hWnd: HWND, lpString: *mut u16, nMaxCount: i32) -> i32;
        fn GetWindowThreadProcessId(hWnd: HWND, lpdwProcessId: *mut DWORD) -> DWORD;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(dwDesiredAccess: DWORD, bInheritHandle: BOOL, dwProcessId: DWORD) -> *mut std::ffi::c_void;
        fn QueryFullProcessImageNameW(hProcess: *mut std::ffi::c_void, dwFlags: DWORD, lpExeName: *mut u16, lpdwSize: *mut DWORD) -> BOOL;
        fn CloseHandle(hObject: *mut std::ffi::c_void) -> BOOL;
    }

    let mut windows: Vec<WindowInfo> = Vec::new();

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        if unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }

        let len = unsafe { GetWindowTextLengthW(hwnd) };
        if len <= 0 {
            return 1;
        }

        let mut title_buf = vec![0u16; (len + 1) as usize];
        let copied = unsafe { GetWindowTextW(hwnd, title_buf.as_mut_ptr(), len + 1) };
        if copied <= 0 {
            return 1;
        }

        let title = String::from_utf16_lossy(&title_buf[..copied as usize]);
        if title == "Program Manager" || title == "Settings" || title == "Specter" {
            return 1;
        }

        let mut pid: DWORD = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };

        let mut exe_name = String::new();
        let proc_handle = unsafe { OpenProcess(0x1000, 0, pid) };
        if !proc_handle.is_null() {
            let mut name_buf = vec![0u16; 1024];
            let mut size: DWORD = 1024;
            if unsafe { QueryFullProcessImageNameW(proc_handle, 0, name_buf.as_mut_ptr(), &mut size) } != 0 {
                let full_path = String::from_utf16_lossy(&name_buf[..size as usize]);
                if let Some(name) = std::path::Path::new(&full_path).file_name() {
                    exe_name = name.to_string_lossy().to_string();
                }
            }
            unsafe { CloseHandle(proc_handle) };
        }

        let list = unsafe { &mut *(lparam as *mut Vec<WindowInfo>) };
        list.push(WindowInfo {
            pid,
            title,
            exe: exe_name,
        });

        1
    }

    unsafe {
        EnumWindows(enum_proc, &mut windows as *mut _ as LPARAM);
    }

    windows
}

#[cfg(not(windows))]
pub fn enumerate_windows() -> Vec<WindowInfo> {
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_engine_idle_lifecycle() {
        let engine = CaptureEngine::new();
        let stats = engine.get_stats();
        assert_eq!(stats.used_mb, 0.0);
        assert_eq!(stats.duration_seconds, 0.0);
        assert!(engine.stop().is_ok());
    }

    #[test]
    fn test_capture_engine_buffer_lifecycle_and_save_continue() {
        let engine = CaptureEngine::new();
        let mut settings = Settings::default();
        let temp_dir = std::env::temp_dir().join("specter_test_save_continue");
        let _ = std::fs::remove_dir_all(&temp_dir);
        settings.output_folder = temp_dir.to_string_lossy().to_string();

        {
            let mut b = engine.buffer.lock().unwrap();
            *b = Some(RingBuffer::new(settings.ram_limit_mb, settings.buffer_minutes));
        }
        {
            let mut s = engine.settings.lock().unwrap();
            *s = Some(settings);
        }

        {
            let mut guard = engine.buffer.lock().unwrap();
            let buf = guard.as_mut().unwrap();
            buf.push(b"ftypisommoovheadermoof".to_vec(), 1000.0);
            buf.push(b"moof_fragment_1".to_vec(), 1001.0);
            buf.push(b"moof_fragment_2".to_vec(), 1003.5);
        }

        let stats = engine.get_stats();
        assert!(stats.used_mb > 0.0);
        assert_eq!(stats.duration_seconds, 2.5);

        let start_res = engine.save_clip(true).expect("First Save + Continue must start disk recording");
        assert!(start_res.starts_with("RECORDING_STARTED:"));
        assert!(engine.is_recording_to_disk());

        let stats_rec = engine.get_stats();
        assert!(stats_rec.is_recording_to_disk);
        assert!(stats_rec.duration_seconds >= 2.5);

        let saved_path = engine.save_clip(true).expect("Second Save + Continue must stop recording and finalize file");
        assert!(!engine.is_recording_to_disk());
        assert!(std::path::Path::new(&saved_path).exists());
        let file_len = std::fs::metadata(&saved_path).unwrap().len();
        assert!(file_len > 0);

        let stats_after = engine.get_stats();
        assert!(!stats_after.is_recording_to_disk);
        assert!(engine.buffer.lock().unwrap().is_some());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_capture_engine_save_and_stop() {
        let engine = CaptureEngine::new();
        let mut settings = Settings::default();
        let temp_dir = std::env::temp_dir().join("specter_test_save_stop");
        let _ = std::fs::remove_dir_all(&temp_dir);
        settings.output_folder = temp_dir.to_string_lossy().to_string();

        {
            let mut b = engine.buffer.lock().unwrap();
            *b = Some(RingBuffer::new(settings.ram_limit_mb, settings.buffer_minutes));
        }
        {
            let mut s = engine.settings.lock().unwrap();
            *s = Some(settings);
        }

        {
            let mut guard = engine.buffer.lock().unwrap();
            let buf = guard.as_mut().unwrap();
            buf.push(b"ftypisommoovheader".to_vec(), 100.0);
            buf.push(b"moof_chunk_1".to_vec(), 102.0);
        }

        let saved_path = engine.save_clip(false).expect("Save + Stop must succeed");
        assert!(std::path::Path::new(&saved_path).exists());
        assert!(engine.buffer.lock().unwrap().is_none());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
