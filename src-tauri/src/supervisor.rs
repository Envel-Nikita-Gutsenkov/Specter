use std::process::{Child, Command, Stdio};
use anyhow::{Context, Result};
use crate::settings::Settings;

pub struct ProcessSupervisor {
    pub child: Option<Child>,
}

impl ProcessSupervisor {
    pub fn new() -> Self {
        Self { child: None }
    }

    pub fn resolve_ffmpeg_binary() -> std::path::PathBuf {
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(parent) = exe_path.parent() {
                let candidate = parent.join("ffmpeg.exe");
                if candidate.exists() {
                    return candidate;
                }
                let bin_candidate = parent.join("bin").join("ffmpeg.exe");
                if bin_candidate.exists() {
                    return bin_candidate;
                }
            }
        }

        #[cfg(windows)]
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let candidate = std::path::PathBuf::from(local_app_data).join("Specter").join("bin").join("ffmpeg.exe");
            if candidate.exists() {
                return candidate;
            }
        }

        std::path::PathBuf::from("ffmpeg")
    }

    pub fn spawn(settings: &Settings) -> Result<(Self, std::process::ChildStdout)> {
        let args = Self::build_ffmpeg_args(settings);

        let log_file = std::env::temp_dir().join("specter_ffmpeg.log");
        let stderr_dest = match std::fs::File::create(&log_file) {
            Ok(f) => Stdio::from(f),
            Err(_) => Stdio::null(),
        };

        let ffmpeg_bin = Self::resolve_ffmpeg_binary();
        let mut cmd = Command::new(&ffmpeg_bin);
        cmd.args(&args)
            .stdout(Stdio::piped())
            .stderr(stderr_dest);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        let mut child = cmd.spawn().context("Failed to spawn FFmpeg process. Is FFmpeg installed and in PATH?")?;
        let stdout = child.stdout.take().context("Failed to capture FFmpeg stdout pipe")?;

        Ok((Self { child: Some(child) }, stdout))
    }

    pub fn terminate(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn is_alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => match c.try_wait() {
                Ok(None) => true,
                _ => false,
            },
            None => false,
        }
    }

    pub fn build_ffmpeg_args(s: &Settings) -> Vec<String> {
        let fps_str = s.fps.to_string();
        let scale_filter = match s.resolution.as_str() {
            "1080p" => Some("scale=1920:1080"),
            "1440p" => Some("scale=2560:1440"),
            "720p"  => Some("scale=1280:720"),
            _       => None,
        };

        let bitrate = s.bitrate_mbps
            .map(|b| format!("{}M", b))
            .unwrap_or_else(|| "6M".to_string());

        let mut args = Vec::new();

        #[cfg(windows)]
        {
            args.extend(["-f".to_string(), "gdigrab".to_string()]);
            args.extend(["-framerate".to_string(), fps_str.clone()]);

            if s.capture_source.starts_with("screen:") {
                let idx: usize = s.capture_source["screen:".len()..].parse().unwrap_or(0);
                let rects = crate::capture::enumerate_monitors_with_rect();
                if let Some(r) = rects.get(idx) {
                    args.extend([
                        "-offset_x".to_string(), r.left.to_string(),
                        "-offset_y".to_string(), r.top.to_string(),
                        "-video_size".to_string(), format!("{}x{}", r.width, r.height),
                    ]);
                }
                args.extend(["-i".to_string(), "desktop".to_string()]);
            } else if s.capture_source.starts_with("window:") {
                let pid_str = &s.capture_source["window:".len()..];
                let pid: u32 = pid_str.parse().unwrap_or(0);
                let windows = crate::capture::enumerate_windows();
                if let Some(w) = windows.iter().find(|w| w.pid == pid) {
                    args.extend(["-i".to_string(), format!("title={}", w.title)]);
                } else {
                    args.extend(["-i".to_string(), "desktop".to_string()]);
                }
            } else {
                args.extend(["-i".to_string(), "desktop".to_string()]);
            }

            if s.audio == "loopback" {
                args.extend([
                    "-f".to_string(), "dshow".to_string(),
                    "-i".to_string(), r#"audio=WASAPI loopback"#.to_string(),
                ]);
            }
        }

        #[cfg(not(windows))]
        {
            args.extend(["-f".to_string(), "x11grab".to_string()]);
            args.extend(["-framerate".to_string(), fps_str.clone()]);
            let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0.0".to_string());
            args.extend(["-i".to_string(), display]);

            if s.audio == "loopback" {
                args.extend([
                    "-f".to_string(), "pulse".to_string(),
                    "-i".to_string(), "default".to_string(),
                ]);
            }
        }

        if s.codec.contains("nvenc") {
            args.extend([
                "-vcodec".to_string(), s.codec.clone(),
                "-pix_fmt".to_string(), "yuv420p".to_string(),
                "-preset".to_string(), "p4".to_string(),
                "-tune".to_string(), "ll".to_string(),
                "-g".to_string(), fps_str.clone(),
                "-strict_gop".to_string(), "1".to_string(),
                "-no-scenecut".to_string(), "1".to_string(),
                "-forced-idr".to_string(), "1".to_string(),
                "-b:v".to_string(), bitrate,
            ]);
        } else if s.codec.contains("amf") {
            args.extend([
                "-vcodec".to_string(), s.codec.clone(),
                "-pix_fmt".to_string(), "yuv420p".to_string(),
                "-quality".to_string(), "speed".to_string(),
                "-g".to_string(), fps_str.clone(),
                "-strict_gop".to_string(), "1".to_string(),
                "-b:v".to_string(), bitrate,
            ]);
        } else if s.codec.contains("qsv") {
            args.extend([
                "-vcodec".to_string(), s.codec.clone(),
                "-pix_fmt".to_string(), "yuv420p".to_string(),
                "-preset".to_string(), "veryfast".to_string(),
                "-g".to_string(), fps_str.clone(),
                "-strict_gop".to_string(), "1".to_string(),
                "-b:v".to_string(), bitrate,
            ]);
        } else {
            args.extend([
                "-vcodec".to_string(), "libx264".to_string(),
                "-pix_fmt".to_string(), "yuv420p".to_string(),
                "-preset".to_string(), "ultrafast".to_string(),
                "-tune".to_string(), "zerolatency".to_string(),
                "-g".to_string(), fps_str.clone(),
                "-sc_threshold".to_string(), "0".to_string(),
                "-strict_gop".to_string(), "1".to_string(),
                "-b:v".to_string(), bitrate,
            ]);
        }

        if let Some(filter) = scale_filter {
            args.extend(["-vf".to_string(), filter.to_string()]);
        }

        if s.audio == "loopback" {
            args.extend([
                "-acodec".to_string(), "aac".to_string(),
                "-b:a".to_string(), "192k".to_string(),
            ]);
        }

        args.extend([
            "-f".to_string(), "mp4".to_string(),
            "-movflags".to_string(), "frag_keyframe+empty_moov+default_base_moof".to_string(),
            "pipe:1".to_string(),
        ]);

        args
    }
}

impl Drop for ProcessSupervisor {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ffmpeg_nvenc_args() {
        let mut s = Settings::default();
        s.codec = "h264_nvenc".to_string();
        let args = ProcessSupervisor::build_ffmpeg_args(&s);
        assert!(args.contains(&"-vcodec".to_string()));
        assert!(args.contains(&"h264_nvenc".to_string()));
        assert!(args.contains(&"-pix_fmt".to_string()));
        assert!(args.contains(&"yuv420p".to_string()));
        assert!(args.contains(&"-g".to_string()));
        assert!(args.contains(&"-preset".to_string()));
        assert!(args.contains(&"p4".to_string()));
    }

    #[test]
    fn test_ffmpeg_hevc_nvenc_args() {
        let mut s = Settings::default();
        s.codec = "hevc_nvenc".to_string();
        let args = ProcessSupervisor::build_ffmpeg_args(&s);
        assert!(args.contains(&"-vcodec".to_string()));
        assert!(args.contains(&"hevc_nvenc".to_string()));
        assert!(args.contains(&"-pix_fmt".to_string()));
        assert!(args.contains(&"yuv420p".to_string()));
        assert!(args.contains(&"-g".to_string()));
    }

    #[test]
    fn test_ffmpeg_x264_args() {
        let mut s = Settings::default();
        s.codec = "libx264".to_string();
        let args = ProcessSupervisor::build_ffmpeg_args(&s);
        assert!(args.contains(&"-vcodec".to_string()));
        assert!(args.contains(&"libx264".to_string()));
        assert!(args.contains(&"-pix_fmt".to_string()));
        assert!(args.contains(&"yuv420p".to_string()));
        assert!(args.contains(&"-preset".to_string()));
        assert!(args.contains(&"ultrafast".to_string()));
    }
}
