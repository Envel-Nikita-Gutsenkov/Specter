use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub buffer_minutes: u32,
    pub ram_limit_mb: u32,
    pub resolution: String,
    pub fps: u32,
    pub codec: String,
    pub bitrate_mbps: Option<f32>,
    pub audio: String,
    pub output_folder: String,
    pub hotkey_toggle_buffer: String,
    pub hotkey_save_continue: String,
    pub hotkey_save_stop: String,
    pub capture_source: String,
    pub capture_cursor: bool,
    pub start_minimized: bool,
    pub show_overlay: bool,
    pub minimize_to_tray: bool,
    pub recording_mode: String,
    pub recording_resolution: String,
    pub recording_fps: u32,
    pub recording_audio: String,
    pub recording_quality: String,
    pub recording_codec: String,
    pub mic_device: String,
    /// Capture backend: `auto` (Desktop Duplication when available), `dda`, or
    /// `gdi`. Exposed so a machine where DDA misbehaves can force GDI without a
    /// rebuild.
    pub capture_backend: String,
    /// Encoder speed/quality trade-off: `speed`, `balanced`, or `quality`.
    pub video_preset: String,
}

impl Default for Settings {
    fn default() -> Self {
        let output_folder = dirs::video_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Specter")
            .to_string_lossy()
            .to_string();

        Self {
            buffer_minutes: 5,
            ram_limit_mb: 500,
            resolution: "1080p".to_string(),
            fps: 60,
            codec: "h264_nvenc".to_string(),
            bitrate_mbps: None,
            audio: "none".to_string(),
            output_folder,
            hotkey_toggle_buffer: "Alt+Shift+F8".to_string(),
            hotkey_save_continue: "F9".to_string(),
            hotkey_save_stop: "F10".to_string(),
            capture_source: "screen:0".to_string(),
            capture_cursor: false,
            start_minimized: false,
            show_overlay: true,
            minimize_to_tray: false,
            recording_mode: "video".to_string(),
            recording_resolution: "1080p".to_string(),
            recording_fps: 60,
            recording_audio: "none".to_string(),
            recording_quality: "high".to_string(),
            recording_codec: "h264_nvenc".to_string(),
            mic_device: String::new(),
            capture_backend: "auto".to_string(),
            video_preset: "balanced".to_string(),
        }
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("specter")
        .join("config.json")
}

pub fn load() -> Settings {
    let path = config_path();
    if let Ok(data) = fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Settings::default()
    }
}

pub fn save(settings: &Settings) -> anyhow::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings)?;
    fs::write(&path, json)?;
    Ok(())
}
