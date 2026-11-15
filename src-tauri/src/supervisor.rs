use crate::settings::Settings;
use anyhow::{bail, Context, Result};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

#[cfg(windows)]
use crate::loopback::{self, SystemAudioLoopback};

pub struct ProcessSupervisor {
    child: Option<Child>,
    /// Loopback capture feeding the replay buffer's system audio track.
    #[cfg(windows)]
    system_audio: Option<SystemAudioLoopback>,
}

/// Translates the user-facing speed/quality preference into the preset name of
/// the selected encoder family. Keeping this in one place makes the Record tab
/// and the replay buffer behave identically.
pub fn encoder_preset(codec: &str, preference: &str) -> &'static str {
    if codec.contains("nvenc") {
        match preference {
            "speed" => "p1",
            "quality" => "p6",
            _ => "p4",
        }
    } else if codec.contains("amf") {
        match preference {
            "speed" => "speed",
            "quality" => "quality",
            _ => "balanced",
        }
    } else if codec.contains("qsv") {
        match preference {
            "speed" => "veryfast",
            "quality" => "slow",
            _ => "medium",
        }
    } else if codec.contains("265") {
        match preference {
            "speed" => "ultrafast",
            "quality" => "medium",
            _ => "faster",
        }
    } else {
        match preference {
            "speed" => "ultrafast",
            "quality" => "faster",
            _ => "veryfast",
        }
    }
}

/// Returns the tail of a log so a failed FFmpeg start reports the actual error.
fn tail_log(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if count <= max_chars {
        return trimmed.to_string();
    }
    trimmed.chars().skip(count - max_chars).collect()
}

impl ProcessSupervisor {
    pub fn new() -> Self {
        Self {
            child: None,
            #[cfg(windows)]
            system_audio: None,
        }
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
            let candidate = std::path::PathBuf::from(local_app_data)
                .join("Specter")
                .join("bin")
                .join("ffmpeg.exe");
            if candidate.exists() {
                return candidate;
            }
        }

        std::path::PathBuf::from("ffmpeg")
    }

    pub fn spawn(settings: &Settings) -> Result<(Self, std::process::ChildStdout)> {
        // System audio is captured over WASAPI and arrives as raw PCM through a
        // named pipe; the pipe has to exist before FFmpeg opens its inputs.
        #[cfg(windows)]
        let system_audio = if settings.audio == "loopback" {
            Some(SystemAudioLoopback::start().context("Failed to capture system audio")?)
        } else {
            None
        };
        #[cfg(windows)]
        let loopback_path = system_audio.as_ref().map(|capture| capture.path());
        #[cfg(not(windows))]
        let loopback_path: Option<&str> = None;

        let args = Self::build_ffmpeg_args_with_audio(settings, loopback_path);

        let log_file = std::env::temp_dir().join("specter_ffmpeg.log");
        let stderr_dest = match std::fs::File::create(&log_file) {
            Ok(f) => Stdio::from(f),
            Err(_) => Stdio::null(),
        };

        let ffmpeg_bin = Self::resolve_ffmpeg_binary();
        let mut cmd = Command::new(&ffmpeg_bin);
        cmd.args(&args).stdout(Stdio::piped()).stderr(stderr_dest);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        let mut child = cmd
            .spawn()
            .context("Failed to spawn FFmpeg process. Is FFmpeg installed and in PATH?")?;
        let stdout = child
            .stdout
            .take()
            .context("Failed to capture FFmpeg stdout pipe")?;

        // Surface early failures (missing filter/device/encoder) instead of leaving
        // the replay engine in a fake "running" state that captures nothing.
        std::thread::sleep(Duration::from_millis(300));
        if let Some(status) = child.try_wait()? {
            let log = std::fs::read_to_string(&log_file).unwrap_or_default();
            bail!(
                "FFmpeg exited immediately ({}). {}",
                status,
                tail_log(&log, 700)
            );
        }

        Ok((
            Self {
                child: Some(child),
                #[cfg(windows)]
                system_audio,
            },
            stdout,
        ))
    }

    pub fn terminate(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        #[cfg(windows)]
        {
            self.system_audio.take();
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
        Self::build_ffmpeg_args_with_audio(s, None)
    }

    /// `loopback_path` is the named pipe that carries the WASAPI loopback audio.
    /// FFmpeg cannot open the system audio device itself, so without a path the
    /// replay buffer records video only.
    pub fn build_ffmpeg_args_with_audio(s: &Settings, loopback_path: Option<&str>) -> Vec<String> {
        let fps_str = s.fps.to_string();
        let scale_filter = match s.resolution.as_str() {
            "1080p" => Some("scale=1920:1080"),
            "1440p" => Some("scale=2560:1440"),
            "720p" => Some("scale=1280:720"),
            _ => None,
        };

        let bitrate = s
            .bitrate_mbps
            .map(|b| format!("{}M", b))
            .unwrap_or_else(|| "6M".to_string());

        let mut args = Vec::new();
        let hardware_frames;

        #[cfg(windows)]
        {
            // The audio device is opened before the capture input: a stalled audio
            // read in FFmpeg's demux loop would otherwise cost most of the desktop
            // frames, because `ddagrab` only exposes the newest one.
            if let Some(path) = loopback_path.filter(|_| s.audio == "loopback") {
                args.extend([
                    "-f".to_string(),
                    "s16le".to_string(),
                    "-ar".to_string(),
                    loopback::SAMPLE_RATE.to_string(),
                    "-ac".to_string(),
                    loopback::CHANNELS.to_string(),
                    "-i".to_string(),
                    path.to_string(),
                ]);
            }

            // Prefer Desktop Duplication (`ddagrab`) for monitors and fall back to
            // GDI (`gdigrab`) for windows or FFmpeg builds without the filter.
            let capture = crate::capture::desktop_capture_args(
                &s.capture_source,
                s.fps,
                s.capture_cursor,
                &s.capture_backend,
            );
            hardware_frames = capture.hardware_frames;
            args.extend(capture.args);
        }

        #[cfg(not(windows))]
        {
            hardware_frames = false;
            if s.audio == "loopback" {
                args.extend([
                    "-f".to_string(),
                    "pulse".to_string(),
                    "-i".to_string(),
                    "default".to_string(),
                ]);
            }

            args.extend(["-f".to_string(), "x11grab".to_string()]);
            args.extend(["-framerate".to_string(), fps_str.clone()]);
            let draw_mouse = if s.capture_cursor { "1" } else { "0" };
            args.extend(["-draw_mouse".to_string(), draw_mouse.to_string()]);
            let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0.0".to_string());
            args.extend(["-i".to_string(), display]);
        }

        // Every audio input precedes the capture input, so the capture index is the
        // number of audio tracks opened above.
        #[cfg(windows)]
        let audio_tracks = if s.audio == "loopback" && loopback_path.is_some() {
            1
        } else {
            0
        };
        #[cfg(not(windows))]
        let audio_tracks = if s.audio == "loopback" { 1 } else { 0 };
        if s.audio == "loopback" && audio_tracks == 0 {
            args.extend(["-an".to_string()]);
        }
        if audio_tracks > 0 {
            args.extend([
                "-map".to_string(),
                format!("{}:v", audio_tracks),
                "-map".to_string(),
                "0:a".to_string(),
            ]);
        }

        if s.codec.contains("nvenc") {
            args.extend([
                "-vcodec".to_string(),
                s.codec.clone(),
                "-pix_fmt".to_string(),
                "yuv420p".to_string(),
                "-preset".to_string(),
                encoder_preset(&s.codec, &s.video_preset).to_string(),
                "-tune".to_string(),
                "ll".to_string(),
                "-bf".to_string(),
                "0".to_string(),
                "-g".to_string(),
                fps_str.clone(),
                "-strict_gop".to_string(),
                "1".to_string(),
                "-no-scenecut".to_string(),
                "1".to_string(),
                "-forced-idr".to_string(),
                "1".to_string(),
                "-b:v".to_string(),
                bitrate,
            ]);
        } else if s.codec.contains("amf") {
            args.extend([
                "-vcodec".to_string(),
                s.codec.clone(),
                "-pix_fmt".to_string(),
                "yuv420p".to_string(),
                "-quality".to_string(),
                encoder_preset(&s.codec, &s.video_preset).to_string(),
                "-bf".to_string(),
                "0".to_string(),
                "-g".to_string(),
                fps_str.clone(),
                "-strict_gop".to_string(),
                "1".to_string(),
                "-b:v".to_string(),
                bitrate,
            ]);
        } else if s.codec.contains("qsv") {
            args.extend([
                "-vcodec".to_string(),
                s.codec.clone(),
                "-pix_fmt".to_string(),
                "yuv420p".to_string(),
                "-preset".to_string(),
                encoder_preset(&s.codec, &s.video_preset).to_string(),
                "-bf".to_string(),
                "0".to_string(),
                "-g".to_string(),
                fps_str.clone(),
                "-strict_gop".to_string(),
                "1".to_string(),
                "-b:v".to_string(),
                bitrate,
            ]);
        } else {
            args.extend([
                "-vcodec".to_string(),
                "libx264".to_string(),
                "-pix_fmt".to_string(),
                "yuv420p".to_string(),
                "-preset".to_string(),
                encoder_preset(&s.codec, &s.video_preset).to_string(),
                "-tune".to_string(),
                "zerolatency".to_string(),
                "-bf".to_string(),
                "0".to_string(),
                "-g".to_string(),
                fps_str.clone(),
                "-sc_threshold".to_string(),
                "0".to_string(),
                "-strict_gop".to_string(),
                "1".to_string(),
                "-b:v".to_string(),
                bitrate,
            ]);
        }

        let mut filters: Vec<String> = Vec::new();
        if hardware_frames {
            filters.push("hwdownload,format=bgra".to_string());
        }
        if let Some(filter) = scale_filter {
            filters.push(filter.to_string());
        }
        if hardware_frames {
            filters.push("format=yuv420p".to_string());
        }
        // Normalise Desktop Duplication's update-driven variable timestamps to a
        // constant rate; without this the saved replay can have long timestamp
        // gaps that make players (and hardware decoders) misbehave.
        filters.push(format!("fps={}", fps_str));
        if !filters.is_empty() {
            args.extend(["-vf".to_string(), filters.join(",")]);
        }

        if audio_tracks > 0 {
            args.extend([
                "-acodec".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                "192k".to_string(),
            ]);
        }

        args.extend([
            "-f".to_string(),
            "mp4".to_string(),
            "-movflags".to_string(),
            "frag_keyframe+empty_moov+default_base_moof".to_string(),
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
        s.video_preset = "speed".to_string();
        let args = ProcessSupervisor::build_ffmpeg_args(&s);
        assert!(args.contains(&"-vcodec".to_string()));
        assert!(args.contains(&"libx264".to_string()));
        assert!(args.contains(&"-pix_fmt".to_string()));
        assert!(args.contains(&"yuv420p".to_string()));
        assert!(args.contains(&"-preset".to_string()));
        assert!(args.contains(&"ultrafast".to_string()));
    }

    #[test]
    fn test_video_preset_maps_per_encoder_family() {
        assert_eq!(encoder_preset("h264_nvenc", "speed"), "p1");
        assert_eq!(encoder_preset("h264_nvenc", "balanced"), "p4");
        assert_eq!(encoder_preset("h264_nvenc", "quality"), "p6");
        assert_eq!(encoder_preset("h264_amf", "quality"), "quality");
        assert_eq!(encoder_preset("h264_qsv", "speed"), "veryfast");
        assert_eq!(encoder_preset("libx264", "balanced"), "veryfast");
        assert_eq!(encoder_preset("libx265", "balanced"), "faster");
    }

    fn draw_mouse_value(args: &[String]) -> Option<String> {
        for (index, arg) in args.iter().enumerate() {
            if arg == "-draw_mouse" {
                return args.get(index + 1).cloned();
            }
            if let Some(rest) = arg.split("draw_mouse=").nth(1) {
                return Some(rest.split(':').next().unwrap_or("").to_string());
            }
        }
        None
    }

    #[test]
    fn test_ffmpeg_cursor_capture_flag() {
        let mut s = Settings::default();
        s.capture_cursor = false;
        let args = ProcessSupervisor::build_ffmpeg_args(&s);
        assert_eq!(draw_mouse_value(&args).as_deref(), Some("0"));

        s.capture_cursor = true;
        let args2 = ProcessSupervisor::build_ffmpeg_args(&s);
        assert_eq!(draw_mouse_value(&args2).as_deref(), Some("1"));
    }
}
