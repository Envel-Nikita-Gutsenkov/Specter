use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::settings::Settings;
use crate::storage::StorageManager;
use crate::supervisor::ProcessSupervisor;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// FFmpeg audio filter that computes per-frame RMS and logs it to stderr.
/// `direct=1` is required: without it the metadata is buffered until the
/// process exits and the waveform would only update after stopping.
/// Output lines look like `... lavfi.astats.Overall.RMS_level=-23.4`.
pub const AUDIO_METER_FILTER: &str =
    "astats=metadata=1:reset=1,ametadata=mode=print:key=lavfi.astats.Overall.RMS_level:direct=1";

/// Audio processing applied before metering. `aresample=async` stretches or pads
/// by whole samples to keep audio locked to the wall clock, so audio continues
/// cleanly even when the video capture path drops frames or stalls. Audio is the
/// source of truth for sync: video frames may be lost, audio must not be.
fn audio_processing_filter() -> String {
    format!("aresample=async=1:first_pts=0,{}", AUDIO_METER_FILTER)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RecordingConfig {
    pub mode: String,
    pub resolution: String,
    pub fps: u32,
    pub quality: String,
    pub audio: String,
    pub mic_device: String,
    pub codec: String,
    pub capture_source: String,
    pub capture_cursor: bool,
    pub capture_backend: String,
    pub video_preset: String,
    pub output_folder: String,
}

impl RecordingConfig {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            mode: settings.recording_mode.clone(),
            resolution: settings.recording_resolution.clone(),
            fps: settings.recording_fps,
            quality: settings.recording_quality.clone(),
            audio: settings.recording_audio.clone(),
            mic_device: settings.mic_device.clone(),
            codec: settings.recording_codec.clone(),
            capture_source: settings.capture_source.clone(),
            capture_cursor: settings.capture_cursor,
            capture_backend: settings.capture_backend.clone(),
            video_preset: settings.video_preset.clone(),
            output_folder: settings.output_folder.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RecorderStats {
    pub is_recording: bool,
    pub duration_seconds: f64,
    pub size_mb: f64,
    pub level: f32,
    pub mode: String,
}

pub struct RecorderEngine {
    child: Mutex<Option<Child>>,
    reader: Mutex<Option<JoinHandle<()>>>,
    segments: Mutex<Vec<PathBuf>>,
    current_temp: Mutex<Option<PathBuf>>,
    final_path: Mutex<Option<PathBuf>>,
    base_name: Mutex<String>,
    started_at: Mutex<Option<f64>>,
    mode: Mutex<String>,
    level: Arc<Mutex<f32>>,
    /// True if any segment carried a real audio source. Video segments always
    /// include an (possibly silent) audio stream so they concatenate cleanly; if
    /// no real source was ever chosen, that silent track is stripped on stop.
    had_audio: Mutex<bool>,
}

impl RecorderEngine {
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
            reader: Mutex::new(None),
            segments: Mutex::new(Vec::new()),
            current_temp: Mutex::new(None),
            final_path: Mutex::new(None),
            base_name: Mutex::new(String::new()),
            started_at: Mutex::new(None),
            mode: Mutex::new(String::new()),
            level: Arc::new(Mutex::new(0.0)),
            had_audio: Mutex::new(false),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.child.lock().unwrap().is_some()
    }

    pub fn start(&self, config: &RecordingConfig, app: AppHandle) -> Result<()> {
        if self.is_recording() {
            bail!("A recording is already in progress");
        }
        if config.mode != "video" && config.mode != "audio" {
            bail!("Unknown recording mode: {}", config.mode);
        }
        if config.mode == "audio" && !is_audio_enabled(&config.audio) {
            bail!("Audio-only recording requires a microphone or system audio source");
        }

        let folder = StorageManager::resolve_output_folder(&config.output_folder);
        fs::create_dir_all(&folder).context("Failed to create recording directory")?;

        let base_name = format!("Specter_{}", StorageManager::format_timestamp());
        let extension = if config.mode == "audio" { "m4a" } else { "mp4" };
        let final_path = folder.join(format!("{}.{}", base_name, extension));
        // Capture into Matroska: it is written incrementally and stays playable even
        // if the process is killed, unlike fragmented MP4 which needs a keyframe.
        let temp_ext = if config.mode == "audio" { "mka" } else { "mkv" };
        let temp_path = folder.join(segment_file_name(&base_name, temp_ext, 0));

        self.launch(config, &temp_path, app.clone())?;

        *self.segments.lock().unwrap() = Vec::new();
        *self.current_temp.lock().unwrap() = Some(temp_path);
        *self.final_path.lock().unwrap() = Some(final_path);
        *self.base_name.lock().unwrap() = base_name;
        *self.started_at.lock().unwrap() = Some(current_timestamp());
        *self.mode.lock().unwrap() = config.mode.clone();
        *self.level.lock().unwrap() = 0.0;
        *self.had_audio.lock().unwrap() = is_audio_enabled(&config.audio);

        let _ = app.emit("recorder-status", true);
        Ok(())
    }

    /// Restarts the capture process against a new source. The elapsed timer and the
    /// output file are preserved; segments are concatenated when the recording stops.
    /// The replacement process is started first, so a failed switch never interrupts
    /// the capture that is already running.
    pub fn switch_source(&self, config: &RecordingConfig, app: AppHandle) -> Result<()> {
        if !self.is_recording() {
            bail!("No active recording to switch");
        }

        let final_path = self
            .final_path
            .lock()
            .unwrap()
            .clone()
            .context("Recording state is incomplete")?;
        let base_name = self.base_name.lock().unwrap().clone();
        let temp_ext = if config.mode == "audio" { "mka" } else { "mkv" };
        let next_index = self.segments.lock().unwrap().len() + 1;
        let temp_path = final_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(segment_file_name(&base_name, temp_ext, next_index));

        let (child, reader) =
            Self::spawn(config, &temp_path, app.clone(), Arc::clone(&self.level))?;

        // The new source is live: stop the previous process and keep its output as
        // one more segment for the final concatenation.
        if let Some(mut old) = self.child.lock().unwrap().replace(child) {
            shutdown_process(&mut old);
        }
        let old_reader = {
            let mut guard = self.reader.lock().unwrap();
            std::mem::replace(&mut *guard, reader)
        };
        if let Some(handle) = old_reader {
            let _ = handle.join();
        }
        if let Some(old_temp) = self.current_temp.lock().unwrap().replace(temp_path) {
            if old_temp.exists() {
                self.segments.lock().unwrap().push(old_temp);
            }
        }

        *self.mode.lock().unwrap() = config.mode.clone();
        if is_audio_enabled(&config.audio) {
            *self.had_audio.lock().unwrap() = true;
        }
        let _ = app.emit("recorder-status", true);
        Ok(())
    }

    fn launch(&self, config: &RecordingConfig, temp_path: &Path, app: AppHandle) -> Result<()> {
        let (child, reader) = Self::spawn(config, temp_path, app, Arc::clone(&self.level))?;
        *self.child.lock().unwrap() = Some(child);
        *self.reader.lock().unwrap() = reader;
        Ok(())
    }

    fn spawn(
        config: &RecordingConfig,
        temp_path: &Path,
        app: AppHandle,
        level: Arc<Mutex<f32>>,
    ) -> Result<(Child, Option<JoinHandle<()>>)> {
        let audio_enabled = is_audio_enabled(&config.audio);
        let args = Self::build_args(config, temp_path)?;

        let log_file = std::env::temp_dir().join("specter_recorder_ffmpeg.log");
        let stderr_dest = if audio_enabled {
            Stdio::piped()
        } else {
            match fs::File::create(&log_file) {
                Ok(file) => Stdio::from(file),
                Err(_) => Stdio::null(),
            }
        };

        let ffmpeg_bin = ProcessSupervisor::resolve_ffmpeg_binary();
        let mut cmd = Command::new(&ffmpeg_bin);
        cmd.args(&args)
            .args(["-progress", "pipe:1", "-nostats"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr_dest);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .context("Failed to launch FFmpeg. Is it installed and available in PATH?")?;
        let mut stderr = child.stderr.take();
        let stdout = child.stdout.take();

        // Surface early failures (bad device name, unsupported encoder, ...) instead of
        // leaving the UI in a fake recording state.
        std::thread::sleep(Duration::from_millis(300));
        if let Some(status) = child.try_wait()? {
            let log = if audio_enabled {
                let mut buffer = String::new();
                if let Some(mut pipe) = stderr.take() {
                    let _ = pipe.read_to_string(&mut buffer);
                }
                buffer
            } else {
                fs::read_to_string(&log_file).unwrap_or_default()
            };
            let _ = fs::remove_file(temp_path);
            bail!(
                "FFmpeg exited immediately ({}). {}",
                status,
                tail(&log, 700)
            );
        }

        let mut meter: Option<JoinHandle<()>> = None;
        if let Some(stdout) = stdout {
            Self::spawn_load_monitor(stdout, app.clone(), config.fps);
        }
        if let Some(stderr) = stderr {
            let app_handle = app;
            let log_path = log_file.clone();
            let reader = std::thread::Builder::new()
                .name("specter-audio-meter".to_string())
                .spawn(move || {
                    let mut log = fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&log_path)
                        .ok();
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        let Ok(line) = line else { break };
                        if let Some(pos) = line.find("RMS_level=") {
                            if let Ok(db) = line[pos + "RMS_level=".len()..].trim().parse::<f32>() {
                                // Quiet microphones sit around -45 dB, so widen the
                                // useful range and gamma-lift it for the waveform.
                                let mapped = meter_level(db);
                                *level.lock().unwrap() = mapped;
                                let _ = app_handle.emit("recorder-level", mapped);
                            }
                        } else if let Some(log) = log.as_mut() {
                            let _ = writeln!(log, "{}", line);
                        }
                    }
                });

            match reader {
                Ok(handle) => meter = Some(handle),
                Err(err) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = fs::remove_file(temp_path);
                    return Err(err).context("Failed to spawn audio meter thread");
                }
            }
        }

        Ok((child, meter))
    }

    /// Reads FFmpeg's `-progress` stream and warns when the encoder cannot keep up
    /// with real time. `speed` below 1.0 means frames are being dropped, which is
    /// what happens when a GPU encoder is saturated (for example while Discord also
    /// encodes a screen share). The average `fps` reported by FFmpeg includes start-up
    /// time, so it is not used as a trigger.
    fn spawn_load_monitor(stdout: std::process::ChildStdout, app: AppHandle, target_fps: u32) {
        let target_fps = target_fps.max(1) as f32;
        std::thread::Builder::new()
            .name("specter-recorder-load".to_string())
            .spawn(move || {
                let reader = BufReader::new(stdout);
                let mut fps = 0.0f32;
                let mut speed = 1.0f32;
                let mut frames = 0.0f32;
                let mut duplicated = 0.0f32;
                let mut blocks = 0u32;
                let mut slow_streak = 0u32;
                let mut healthy_streak = 0u32;
                let mut warned = false;
                let mut last_warn: Option<Instant> = None;

                // Let the UI drop a warning left over from a previous segment.
                let _ = app.emit("recorder-warning", String::new());

                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    let Some((key, value)) = line.split_once('=') else {
                        continue;
                    };
                    match key.trim() {
                        "fps" => {
                            if let Ok(parsed) = value.trim().parse::<f32>() {
                                fps = parsed;
                            }
                        }
                        "speed" => {
                            let raw = value.trim().trim_end_matches('x');
                            if let Ok(parsed) = raw.parse::<f32>() {
                                speed = parsed;
                            }
                        }
                        "frame" => {
                            if let Ok(parsed) = value.trim().parse::<f32>() {
                                frames = parsed;
                            }
                        }
                        "dup_frames" => {
                            if let Ok(parsed) = value.trim().parse::<f32>() {
                                duplicated = parsed;
                            }
                        }
                        "progress" => {
                            // The first reports are dominated by codec/capture start-up,
                            // and FFmpeg's `fps` is an average, so it needs several
                            // blocks before it reflects the steady state.
                            blocks += 1;
                            if blocks <= 2 {
                                continue;
                            }

                            // `-fps_mode cfr` inflates the reported fps to the target
                            // rate by duplicating the last frame, so the duplicate
                            // share is what actually tells us whether the capture source
                            // is keeping up.
                            let dup_share = if frames > 0.0 {
                                (duplicated / frames).clamp(0.0, 1.0)
                            } else {
                                0.0
                            };
                            let real_fps = fps * (1.0 - dup_share);

                            // Encoder speed needs a little longer than the capture
                            // rate: the first blocks are dominated by encoder
                            // start-up, where `speed` is meaningless.
                            let encoding_is_late = blocks > 4 && encoder_is_struggling(speed);
                            let low_rate = blocks > 10 && capture_is_starving(real_fps, target_fps);
                            if encoding_is_late || low_rate {
                                slow_streak += 1;
                                healthy_streak = 0;
                                let due = last_warn
                                    .map(|at| at.elapsed() >= Duration::from_secs(10))
                                    .unwrap_or(true);
                                if slow_streak >= 2 && (!warned || due) {
                                    warned = true;
                                    last_warn = Some(Instant::now());
                                    let message = if encoding_is_late {
                                        format!(
                                            "Encoder can't keep up with real time ({:.2}x at {:.0} fps, target {:.0}). \
                                             Frames are being dropped. Close other GPU-heavy apps or \
                                             choose a faster encoder/quality preset.",
                                            speed, fps, target_fps
                                        )
                                    } else {
                                        format!(
                                            "Recording is dropping frames: capturing only {:.0} fps of the \
                                             requested {:.0}. Lower the frame rate, close GPU-heavy apps \
                                             or choose a faster capture backend/preset.",
                                            real_fps, target_fps
                                        )
                                    };
                                    let _ = app.emit("recorder-warning", message);
                                }
                            } else {
                                slow_streak = 0;
                                healthy_streak += 1;
                                if warned && healthy_streak >= 2 {
                                    warned = false;
                                    let _ = app.emit("recorder-warning", String::new());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            })
            .ok();
    }

    /// Stops the running FFmpeg process and keeps its output as one more segment.
    fn halt_current(&self) -> Result<()> {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            shutdown_process(&mut child);
        }
        if let Some(handle) = self.reader.lock().unwrap().take() {
            let _ = handle.join();
        }
        if let Some(temp) = self.current_temp.lock().unwrap().take() {
            if temp.exists() {
                self.segments.lock().unwrap().push(temp);
            }
        }
        Ok(())
    }

    pub fn stop(&self) -> Result<String> {
        if self.final_path.lock().unwrap().is_none() {
            bail!("No active recording to stop");
        }

        self.halt_current()?;

        let final_path = self
            .final_path
            .lock()
            .unwrap()
            .take()
            .context("Recording state is incomplete")?;
        let segments = std::mem::take(&mut *self.segments.lock().unwrap());
        let keep_audio = std::mem::take(&mut *self.had_audio.lock().unwrap());
        self.base_name.lock().unwrap().clear();
        *self.started_at.lock().unwrap() = None;
        self.mode.lock().unwrap().clear();
        *self.level.lock().unwrap() = 0.0;

        if segments.is_empty() {
            bail!("Recording produced no data");
        }

        StorageManager::finalize_recording(&segments, &final_path, keep_audio)
            .map(|path| path.to_string_lossy().to_string())
    }

    pub fn get_stats(&self) -> RecorderStats {
        let is_recording = self.is_recording();
        let duration_seconds = if is_recording {
            self.started_at
                .lock()
                .unwrap()
                .map(|started| (current_timestamp() - started).max(0.0))
                .unwrap_or(0.0)
        } else {
            0.0
        };

        let mut size_bytes: u64 = self
            .segments
            .lock()
            .unwrap()
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|meta| meta.len())
            .sum();
        if let Some(current) = self.current_temp.lock().unwrap().as_ref() {
            if let Ok(meta) = fs::metadata(current) {
                size_bytes += meta.len();
            }
        }

        RecorderStats {
            is_recording,
            duration_seconds,
            size_mb: size_bytes as f64 / (1024.0 * 1024.0),
            level: if is_recording {
                *self.level.lock().unwrap()
            } else {
                0.0
            },
            mode: if is_recording {
                self.mode.lock().unwrap().clone()
            } else {
                String::new()
            },
        }
    }

    pub fn build_args(config: &RecordingConfig, temp_path: &Path) -> Result<Vec<String>> {
        let audio_enabled = is_audio_enabled(&config.audio);
        if config.mode == "audio" && !audio_enabled {
            bail!("Audio-only recording requires a microphone or system audio source");
        }

        let (mic, system) = audio_sources(&config.audio);
        let mic_device = if mic {
            Some(resolve_mic_device(&config.mic_device)?)
        } else {
            None
        };
        let system_device = if system {
            Some(resolve_system_device()?)
        } else {
            None
        };

        Self::build_args_with_devices(
            config,
            temp_path,
            mic_device.as_deref(),
            system_device.as_deref(),
        )
    }

    /// Pure argument builder used by the public API and unit tests.
    pub fn build_args_with_devices(
        config: &RecordingConfig,
        temp_path: &Path,
        mic_device: Option<&str>,
        system_device: Option<&str>,
    ) -> Result<Vec<String>> {
        let audio_enabled = is_audio_enabled(&config.audio);
        let video = config.mode == "video";

        if !video && config.mode != "audio" {
            bail!("Unknown recording mode: {}", config.mode);
        }
        if config.mode == "audio" && !audio_enabled {
            bail!("Audio-only recording requires a microphone or system audio source");
        }

        let fps = config.fps.max(1).to_string();
        let mut args: Vec<String> = Vec::new();

        // Audio inputs are opened *before* the video input on purpose. FFmpeg's
        // demux loop stalls whenever the audio device has nothing queued yet, and
        // `ddagrab` only ever exposes the newest desktop frame — whatever is not
        // pulled in time is lost for good. With the video input first this cost
        // most of the frames (measured: 388 duplicates per 480 frames); with the
        // audio input first the same pipeline drops none (2 per 900).
        let (mic, system) = audio_sources(&config.audio);
        let mut audio_inputs = 0usize;
        if mic {
            let device = mic_device.context("Microphone device is required")?;
            args.extend(audio_input_args(device));
            audio_inputs += 1;
        }
        if system {
            let device = system_device.context("System audio device is required")?;
            args.extend(audio_input_args(device));
            audio_inputs += 1;
        }

        // Video without a real audio source still gets a silent track. Every
        // segment then exposes the same stream layout (one video + one AAC
        // stream), which is required for the concat demuxer to keep the audio
        // track when segments are joined. The silent track is stripped from the
        // final file when no real source was ever used (see `finalize_recording`).
        let silent_audio = video && !audio_enabled;
        if silent_audio {
            args.extend([
                "-f".to_string(),
                "lavfi".to_string(),
                "-i".to_string(),
                "anullsrc=channel_layout=stereo:sample_rate=48000".to_string(),
            ]);
        }

        // Every audio input precedes the video input, so the video input index is
        // the number of audio tracks that were opened above.
        let audio_tracks = if audio_enabled {
            audio_inputs
        } else if silent_audio {
            1
        } else {
            0
        };

        // `ddagrab` produces D3D11 hardware frames that must be downloaded and
        // converted before scaling or a software encoder can use them.
        let hardware_frames = if video {
            append_video_input(&mut args, config, &fps)
        } else {
            false
        };
        let video_input = audio_tracks;

        // Build the single video filter chain used for both the plain `-vf` path
        // and the mixed-audio `-filter_complex` path.
        let video_filter = if video {
            let mut parts: Vec<String> = Vec::new();
            if hardware_frames {
                parts.push("hwdownload".to_string());
                parts.push("format=bgra".to_string());
            }
            if let Some(scale) = scale_filter(&config.resolution) {
                parts.push(scale);
            }
            if hardware_frames {
                // Software encoders and the yuv420p capture contract need a
                // plain system-memory frame after the D3D11 download.
                parts.push("format=yuv420p".to_string());
            }
            // Desktop Duplication only reports a new frame when the screen
            // changes, so its timestamps are variable and can contain gaps of
            // hundreds of milliseconds. The output is forced to a constant frame
            // rate below (`-fps_mode cfr`), which keeps concatenation safe while
            // leaving the duplicate count visible in FFmpeg's progress stream — the
            // load monitor uses it to tell the user when real frames are missing.
            Some(parts.join(","))
        } else {
            None
        };
        let audio_filter = audio_processing_filter();

        if audio_enabled {
            if audio_inputs >= 2 {
                // Only the audio is mixed in the complex graph. Keeping the video
                // path on `-vf` matters: mixing inside the complex graph starved
                // the capture just like reading the video input first did.
                let graph = format!(
                    "[0:a][1:a]amix=inputs=2:duration=longest:normalize=0,{}[aout]",
                    audio_filter
                );
                args.extend(["-filter_complex".to_string(), graph]);
                args.extend(["-map".to_string(), "[aout]".to_string()]);
            } else if audio_inputs == 1 {
                args.extend(["-af".to_string(), audio_filter]);
                args.extend(["-map".to_string(), "0:a".to_string()]);
            }
            if video {
                args.extend(["-map".to_string(), format!("{}:v", video_input)]);
            }
        }

        if video {
            let bitrate = format!(
                "{}M",
                quality_bitrate_mbps(&config.resolution, config.fps, &config.quality)
            );
            append_video_encoder(
                &mut args,
                &config.codec,
                &fps,
                &bitrate,
                &config.video_preset,
            );

            // The video filter always stays on `-vf`, even when the audio is
            // mixed in a complex graph.
            if let Some(filter) = &video_filter {
                args.extend(["-vf".to_string(), filter.clone()]);
            }

            // Constant frame rate with B-frames disabled keeps every segment safe to
            // concatenate. Duplicated frames show up as `dup_frames` in the progress
            // stream, which is exactly the signal the load monitor needs.
            args.extend([
                "-fps_mode".to_string(),
                "cfr".to_string(),
                "-r".to_string(),
                fps.clone(),
            ]);

            if audio_enabled {
                args.extend([
                    "-c:a".to_string(),
                    "aac".to_string(),
                    "-b:a".to_string(),
                    format!("{}k", audio_bitrate_kbps(&config.quality)),
                    "-ar".to_string(),
                    "48000".to_string(),
                    "-ac".to_string(),
                    "2".to_string(),
                ]);
            } else {
                args.extend([
                    "-map".to_string(),
                    format!("{}:v", video_input),
                    "-map".to_string(),
                    "0:a".to_string(),
                    "-c:a".to_string(),
                    "aac".to_string(),
                    "-b:a".to_string(),
                    "64k".to_string(),
                    "-ar".to_string(),
                    "48000".to_string(),
                    "-ac".to_string(),
                    "2".to_string(),
                ]);
            }
        } else {
            args.extend([
                "-vn".to_string(),
                "-ac".to_string(),
                "2".to_string(),
                "-ar".to_string(),
                "48000".to_string(),
                "-c:a".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                format!("{}k", audio_bitrate_kbps(&config.quality)),
            ]);
        }

        args.extend([
            "-f".to_string(),
            "matroska".to_string(),
            "-y".to_string(),
            temp_path.to_string_lossy().to_string(),
        ]);

        Ok(args)
    }
}

impl Default for RecorderEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RecorderEngine {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            shutdown_process(&mut child);
        }
    }
}

/// Asks FFmpeg to finish cleanly (`q` on stdin) so the container trailer is
/// written, then force-kills it if it does not exit in time.
fn shutdown_process(child: &mut Child) {
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"q");
        let _ = stdin.flush();
        drop(stdin);
    }
    for _ in 0..40 {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Temporary file name for one capture segment. `start()` uses index 0, every
/// source switch appends the next index so nothing is overwritten.
fn segment_file_name(base: &str, extension: &str, index: usize) -> String {
    format!(".{}_{}.{}.tmp", base, index, extension)
}

fn current_timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64()
}

fn tail(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let skip = trimmed.chars().count() - max_chars;
    trimmed.chars().skip(skip).collect()
}

fn audio_sources(audio: &str) -> (bool, bool) {
    match audio {
        "mic" => (true, false),
        "system" | "loopback" | "desktop" => (false, true),
        "both" => (true, true),
        _ => (false, false),
    }
}

pub fn is_audio_enabled(audio: &str) -> bool {
    let (mic, system) = audio_sources(audio);
    mic || system
}

/// Maps a raw `astats` RMS value in dB onto the 0..1 range used by the UI.
fn meter_level(db: f32) -> f32 {
    // Quiet microphones sit around -45 dB, so widen the useful range and
    // gamma-lift it for the waveform.
    ((db + 55.0) / 45.0).clamp(0.0, 1.0).powf(0.65)
}

/// The standalone level meter shown before a recording starts. It is a separate
/// FFmpeg process so the UI can prove which device actually produces sound
/// without committing to a full capture.
static IDLE_METER: Mutex<Option<Child>> = Mutex::new(None);

/// FFmpeg arguments for the standalone level meter.
fn idle_meter_args(device: &str) -> Vec<String> {
    let mut args: Vec<String> = vec!["-hide_banner".to_string(), "-nostdin".to_string()];
    args.extend(audio_input_args(device));
    args.extend([
        "-af".to_string(),
        AUDIO_METER_FILTER.to_string(),
        "-f".to_string(),
        "null".to_string(),
        "-".to_string(),
    ]);
    args
}

pub fn stop_idle_meter() {
    if let Ok(mut guard) = IDLE_METER.lock() {
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Starts a low-cost microphone/loopback level monitor that emits
/// `recorder-level` events until stopped.
pub fn start_idle_meter(audio: &str, mic_device: &str, app: AppHandle) -> Result<()> {
    stop_idle_meter();

    let (want_mic, want_system) = audio_sources(audio);
    let device = if want_mic {
        resolve_mic_device(mic_device)?
    } else if want_system {
        resolve_system_device()?
    } else {
        bail!("Select a microphone or system audio source first")
    };

    let args = idle_meter_args(&device);

    let mut cmd = Command::new(ProcessSupervisor::resolve_ffmpeg_binary());
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .context("Failed to start the audio level meter")?;
    let mut stderr = child.stderr.take().context("Missing audio meter stderr")?;

    // Report a bad/unavailable device instead of showing a permanently flat line.
    std::thread::sleep(Duration::from_millis(300));
    if let Some(status) = child.try_wait()? {
        let mut buffer = String::new();
        let _ = stderr.read_to_string(&mut buffer);
        bail!(
            "Audio level meter exited ({}). {}",
            status,
            tail(&buffer, 400)
        );
    }

    std::thread::Builder::new()
        .name("specter-idle-meter".to_string())
        .spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if let Some(pos) = line.find("RMS_level=") {
                    if let Ok(db) = line[pos + "RMS_level=".len()..].trim().parse::<f32>() {
                        let _ = app.emit("recorder-level", meter_level(db));
                    }
                }
            }
            let _ = app.emit("recorder-level", 0.0f32);
        })
        .ok();

    if let Ok(mut guard) = IDLE_METER.lock() {
        *guard = Some(child);
    }
    Ok(())
}

/// Grabs a single downscaled JPEG frame from the requested capture source. Used
/// for the low-rate preview in the Record tab; it never runs while a recording is
/// active so it cannot steal encoder/CPU time from the capture pipeline.
pub fn capture_preview(
    capture_source: &str,
    capture_cursor: bool,
    max_width: u32,
    capture_backend: &str,
) -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        let width = max_width.clamp(160, 1920);

        // Desktop Duplication only reports a frame when the desktop actually
        // changes, so a static screen would never produce a thumbnail. A preview
        // is a single low-rate frame, so GDI is the reliable backend here no
        // matter which backend the recording uses.
        let result = match preview_frame(capture_source, capture_cursor, width, "gdi") {
            Ok(frame) => Ok(frame),
            Err(first) if capture_backend != "gdi" => {
                preview_frame(capture_source, capture_cursor, width, capture_backend)
                    .with_context(|| format!("Preview capture failed ({})", first))
            }
            Err(first) => Err(first),
        };
        result
    }

    #[cfg(not(windows))]
    {
        let _ = (capture_source, capture_cursor, max_width, capture_backend);
        bail!("Capture preview is only supported on Windows")
    }
}

#[cfg(windows)]
fn preview_frame(
    capture_source: &str,
    capture_cursor: bool,
    width: u32,
    capture_backend: &str,
) -> Result<Vec<u8>> {
    use std::os::windows::process::CommandExt;

    let capture =
        crate::capture::desktop_capture_args(capture_source, 30, capture_cursor, capture_backend);
    let mut args: Vec<String> = vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-nostdin".to_string(),
    ];
    args.extend(capture.args);

    let filter = if capture.hardware_frames {
        format!("hwdownload,format=bgra,scale={}:-2,format=yuvj422p", width)
    } else {
        format!("scale={}:-2", width)
    };

    args.extend([
        "-frames:v".to_string(),
        "1".to_string(),
        "-vf".to_string(),
        filter,
        "-q:v".to_string(),
        "6".to_string(),
        "-f".to_string(),
        "image2pipe".to_string(),
        "-vcodec".to_string(),
        "mjpeg".to_string(),
        "pipe:1".to_string(),
    ]);

    let ffmpeg_bin = ProcessSupervisor::resolve_ffmpeg_binary();
    let mut cmd = Command::new(&ffmpeg_bin);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd.spawn().context("Failed to start preview capture")?;
    let mut stdout = child.stdout.take().context("Missing preview stdout")?;
    let mut stderr = child.stderr.take().context("Missing preview stderr")?;

    // Read both pipes on separate threads so FFmpeg can never block on a full
    // pipe while we wait for it to exit.
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().context("Failed to poll preview capture")? {
            let frame = out_reader.join().unwrap_or_default();
            let log = err_reader.join().unwrap_or_default();
            if status.success() && !frame.is_empty() {
                return Ok(frame);
            }
            bail!(
                "preview ffmpeg exited ({}): {}",
                status,
                tail_text(&String::from_utf8_lossy(&log), 300)
            );
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_reader.join();
            let _ = err_reader.join();
            bail!("preview capture timed out");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Returns the tail of a log line so errors stay readable in the UI.
fn tail_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if count <= max_chars {
        trimmed.to_string()
    } else {
        trimmed.chars().skip(count - max_chars).collect()
    }
}

/// True when FFmpeg's `speed` in the `-progress` stream shows the encoder is
/// falling behind real time. `speed` is the output-duration / wall-clock ratio, so
/// values below 1.0 mean input frames are being dropped.
pub fn encoder_is_struggling(speed: f32) -> bool {
    // A realtime capture source never reports a full 1.0x: the last packet is
    // flushed after the source stops, so healthy runs land around 0.93-1.0 even
    // with a hardware encoder that has several times the headroom. Only a clear
    // deficit (the reference machine showed 0.38x while starving) is a real
    // encoder problem; everything else is reported through the duplicate share.
    speed.is_finite() && speed > 0.0 && speed < 0.85
}

/// True when the pipeline only produces a fraction of the requested frame rate.
/// This catches capture starvation, where FFmpeg is fast enough (`speed` >= 1) but
/// the capture source itself delivers far fewer frames than requested, so the
/// recording looks like a slideshow without any encoder warning.
pub fn capture_is_starving(actual_fps: f32, target_fps: f32) -> bool {
    actual_fps.is_finite() && actual_fps >= 0.0 && actual_fps < target_fps * 0.7
}

pub fn quality_bitrate_mbps(resolution: &str, fps: u32, quality: &str) -> f32 {
    let base = match resolution {
        "720p" => 4.0,
        "1440p" => 14.0,
        "1080p" => 7.0,
        _ => 8.0,
    };
    let fps_factor = ((fps.max(1) as f32) / 60.0).sqrt().clamp(0.6, 1.6);
    let quality_factor = match quality {
        "low" => 0.6,
        "medium" => 1.0,
        "high" => 1.6,
        "ultra" => 2.4,
        _ => 1.0,
    };
    let value = base * fps_factor * quality_factor;
    (value * 10.0).round() / 10.0
}

fn audio_bitrate_kbps(quality: &str) -> u32 {
    // Audio is deliberately kept at a high floor: it is cheap compared to video
    // and degrades far more noticeably, so it must not be sacrificed when the
    // video encoder is under pressure.
    match quality {
        "low" => 160,
        "medium" => 192,
        "high" => 256,
        "ultra" => 320,
        _ => 256,
    }
}

fn scale_filter(resolution: &str) -> Option<String> {
    match resolution {
        "1080p" => Some("scale=1920:1080".to_string()),
        "1440p" => Some("scale=2560:1440".to_string()),
        "720p" => Some("scale=1280:720".to_string()),
        _ => None,
    }
}

fn append_video_encoder(
    args: &mut Vec<String>,
    codec: &str,
    fps: &str,
    bitrate: &str,
    preset: &str,
) {
    let preset = crate::supervisor::encoder_preset(codec, preset);
    if codec.contains("nvenc") {
        args.extend([
            "-vcodec".to_string(),
            codec.to_string(),
            "-pix_fmt".to_string(),
            "yuv420p".to_string(),
            "-preset".to_string(),
            preset.to_string(),
            "-tune".to_string(),
            "ll".to_string(),
            "-bf".to_string(),
            "0".to_string(),
            "-g".to_string(),
            fps.to_string(),
            "-strict_gop".to_string(),
            "1".to_string(),
            "-no-scenecut".to_string(),
            "1".to_string(),
            "-forced-idr".to_string(),
            "1".to_string(),
            "-b:v".to_string(),
            bitrate.to_string(),
        ]);
    } else if codec.contains("amf") {
        args.extend([
            "-vcodec".to_string(),
            codec.to_string(),
            "-pix_fmt".to_string(),
            "yuv420p".to_string(),
            "-quality".to_string(),
            preset.to_string(),
            "-bf".to_string(),
            "0".to_string(),
            "-g".to_string(),
            fps.to_string(),
            "-strict_gop".to_string(),
            "1".to_string(),
            "-b:v".to_string(),
            bitrate.to_string(),
        ]);
    } else if codec.contains("qsv") {
        args.extend([
            "-vcodec".to_string(),
            codec.to_string(),
            "-pix_fmt".to_string(),
            "yuv420p".to_string(),
            "-preset".to_string(),
            preset.to_string(),
            "-bf".to_string(),
            "0".to_string(),
            "-g".to_string(),
            fps.to_string(),
            "-strict_gop".to_string(),
            "1".to_string(),
            "-b:v".to_string(),
            bitrate.to_string(),
        ]);
    } else {
        args.extend([
            "-vcodec".to_string(),
            codec.to_string(),
            "-pix_fmt".to_string(),
            "yuv420p".to_string(),
            "-preset".to_string(),
            preset.to_string(),
            "-tune".to_string(),
            "zerolatency".to_string(),
            "-bf".to_string(),
            "0".to_string(),
            "-g".to_string(),
            fps.to_string(),
            "-sc_threshold".to_string(),
            "0".to_string(),
            "-b:v".to_string(),
            bitrate.to_string(),
        ]);
    }
}

#[cfg(windows)]
fn append_video_input(args: &mut Vec<String>, config: &RecordingConfig, fps: &str) -> bool {
    let fps: u32 = fps.parse().unwrap_or(30);
    let capture = crate::capture::desktop_capture_args(
        &config.capture_source,
        fps,
        config.capture_cursor,
        &config.capture_backend,
    );
    args.extend(capture.args);
    capture.hardware_frames
}

#[cfg(not(windows))]
fn append_video_input(args: &mut Vec<String>, _config: &RecordingConfig, fps: &str) -> bool {
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0.0".to_string());
    args.extend([
        "-f".to_string(),
        "x11grab".to_string(),
        "-framerate".to_string(),
        fps.to_string(),
        "-draw_mouse".to_string(),
        "0".to_string(),
        "-i".to_string(),
        display,
    ]);
    false
}

#[cfg(windows)]
fn audio_input_args(device: &str) -> Vec<String> {
    vec![
        "-f".to_string(),
        "dshow".to_string(),
        "-i".to_string(),
        format!("audio={}", device),
    ]
}

#[cfg(not(windows))]
fn audio_input_args(device: &str) -> Vec<String> {
    vec![
        "-f".to_string(),
        "pulse".to_string(),
        "-i".to_string(),
        device.to_string(),
    ]
}

#[cfg(windows)]
fn resolve_mic_device(explicit: &str) -> Result<String> {
    let trimmed = explicit.trim();
    if !trimmed.is_empty() {
        return Ok(trimmed.to_string());
    }
    enumerate_audio_devices()
        .into_iter()
        .next()
        .context("No microphone detected. Select a microphone in the recording panel.")
}

#[cfg(not(windows))]
fn resolve_mic_device(explicit: &str) -> Result<String> {
    let trimmed = explicit.trim();
    Ok(if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    })
}

#[cfg(windows)]
fn resolve_system_device() -> Result<String> {
    enumerate_audio_devices()
        .into_iter()
        .find(|name| is_loopback_name(name))
        .context(
            "No system audio loopback device found. Enable 'Stereo Mix' or install a virtual audio cable.",
        )
}

#[cfg(not(windows))]
fn resolve_system_device() -> Result<String> {
    Ok("default".to_string())
}

fn is_loopback_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        "stereo mix",
        "loopback",
        "virtual-audio-capturer",
        "what u hear",
        "wave out mix",
        "cable output",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[cfg(windows)]
pub fn enumerate_audio_devices() -> Vec<String> {
    use std::os::windows::process::CommandExt;

    let ffmpeg_bin = ProcessSupervisor::resolve_ffmpeg_binary();
    let mut cmd = Command::new(ffmpeg_bin);
    cmd.args([
        "-hide_banner",
        "-list_devices",
        "true",
        "-f",
        "dshow",
        "-i",
        "dummy",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut names: Vec<String> = Vec::new();
    if let Ok(output) = cmd.output() {
        let text = String::from_utf8_lossy(&output.stderr);
        for line in text.lines() {
            if !line.contains("(audio)") {
                continue;
            }
            if let Some(start) = line.find('"') {
                if let Some(end) = line[start + 1..].find('"') {
                    let name = line[start + 1..start + 1 + end].to_string();
                    if !name.is_empty() && !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
        }
    }
    names
}

#[cfg(not(windows))]
pub fn enumerate_audio_devices() -> Vec<String> {
    vec!["default".to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> RecordingConfig {
        RecordingConfig {
            mode: "video".to_string(),
            resolution: "1080p".to_string(),
            fps: 60,
            quality: "high".to_string(),
            audio: "none".to_string(),
            mic_device: String::new(),
            codec: "h264_nvenc".to_string(),
            capture_source: "screen:0".to_string(),
            capture_cursor: false,
            capture_backend: "auto".to_string(),
            video_preset: "balanced".to_string(),
            output_folder: String::new(),
        }
    }

    fn args_for(config: &RecordingConfig) -> Vec<String> {
        RecorderEngine::build_args_with_devices(
            config,
            Path::new("/tmp/specter-test.mp4.tmp"),
            Some("Test Microphone"),
            Some("Test Loopback"),
        )
        .expect("argument builder must succeed")
    }

    #[test]
    fn test_video_args_use_encoder_and_matroska_muxer() {
        let args = args_for(&base_config());
        assert!(args.contains(&"h264_nvenc".to_string()));
        assert!(args.iter().any(|arg| arg.contains("scale=1920:1080")));
        assert!(args.contains(&"matroska".to_string()));
        assert_eq!(args.last().unwrap(), "/tmp/specter-test.mp4.tmp");
    }

    #[test]
    fn test_ddagrab_input_is_downloaded_before_encoding() {
        // When a monitor is captured with `ddagrab`, hardware frames cannot be fed
        // straight into a software encoder: the chain must download them first.
        let config = base_config();
        let capture =
            crate::capture::desktop_capture_args(&config.capture_source, 60, false, "auto");
        if !capture.hardware_frames {
            return; // Machine/FFmpeg build without ddagrab: nothing to assert.
        }
        assert!(capture.args.iter().any(|arg| arg.starts_with("ddagrab=")));
        assert!(!capture.args.iter().any(|arg| arg == "-offset_x"));
        let args = args_for(&config);
        assert!(args
            .iter()
            .any(|arg| arg.starts_with("hwdownload,format=bgra")));
        // The download must be converted to yuv420p and the output must be forced
        // to a constant frame rate before the encoder sees it.
        assert!(args.iter().any(|arg| arg.contains("format=yuv420p")));
        assert!(args.contains(&"-fps_mode".to_string()));
    }

    #[test]
    fn test_video_output_is_constant_rate_without_b_frames() {
        // Variable timestamps from Desktop Duplication plus B-frame reordering are
        // what turned concatenated segments into unplayable files. Segments must be
        // constant-rate and free of reordered frames.
        let args = args_for(&base_config());
        let mode = args
            .iter()
            .position(|arg| arg == "-fps_mode")
            .expect("-fps_mode missing");
        assert_eq!(args[mode + 1], "cfr");
        let rate = args.iter().position(|arg| arg == "-r").expect("-r missing");
        assert_eq!(args[rate + 1], "60");
        let bf = args
            .iter()
            .position(|arg| arg == "-bf")
            .expect("-bf missing");
        assert_eq!(args[bf + 1], "0");
    }

    #[test]
    fn test_silent_video_still_muxes_an_audio_stream() {
        // Every video segment must expose the same stream layout so the concat
        // demuxer keeps audio when segments recorded with different audio settings
        // are joined. Video without a source therefore gets a silent AAC track.
        let args = args_for(&base_config());
        assert!(!args.contains(&"-an".to_string()));
        assert!(args.contains(&"anullsrc=channel_layout=stereo:sample_rate=48000".to_string()));
        let map_positions: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, arg)| arg.as_str() == "-map")
            .map(|(index, _)| index)
            .collect();
        assert_eq!(map_positions.len(), 2);
        assert_eq!(args[map_positions[0] + 1], "1:v");
        assert_eq!(args[map_positions[1] + 1], "0:a");
    }

    #[test]
    fn test_audio_inputs_precede_video_input() {
        // `ddagrab` only exposes the newest desktop frame, so any stall in the
        // demux loop destroys frames. Audio devices routinely leave FFmpeg
        // waiting, therefore every audio input must be opened before the video
        // input. Reading the video input first dropped ~90% of the frames on the
        // reference machine; audio first drops none.
        let mut config = base_config();
        config.audio = "both".to_string();
        let args = args_for(&config);

        let video_input = args
            .iter()
            .position(|arg| arg.starts_with("ddagrab=") || arg == "desktop")
            .expect("video input missing");
        let audio_inputs: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, arg)| arg.starts_with("audio="))
            .map(|(index, _)| index)
            .collect();

        assert_eq!(audio_inputs.len(), 2);
        assert!(
            audio_inputs.iter().all(|index| *index < video_input),
            "audio inputs must be opened before the video input"
        );
        // The video input index follows the two audio inputs.
        assert!(args.contains(&"2:v".to_string()));
    }

    #[test]
    fn test_single_audio_source_precedes_video_input() {
        let mut config = base_config();
        config.audio = "mic".to_string();
        let args = args_for(&config);
        let video_input = args
            .iter()
            .position(|arg| arg.starts_with("ddagrab=") || arg == "desktop")
            .expect("video input missing");
        let audio_input = args
            .iter()
            .position(|arg| arg.starts_with("audio="))
            .expect("audio input missing");
        assert!(audio_input < video_input);
        assert!(args.contains(&"1:v".to_string()));
        assert!(args.contains(&"0:a".to_string()));
    }

    #[test]
    fn test_video_preset_is_applied() {
        let mut config = base_config();
        config.video_preset = "quality".to_string();
        let args = args_for(&config);
        assert!(args.contains(&"p6".to_string()));
    }

    #[test]
    fn test_cpu_hevc_uses_libx265() {
        let mut config = base_config();
        config.codec = "libx265".to_string();
        let args = args_for(&config);
        assert!(args.contains(&"libx265".to_string()));
        assert!(!args.contains(&"libx264".to_string()));
        assert!(args.contains(&"faster".to_string()));
    }

    #[test]
    fn test_audio_only_args_disable_video() {
        let mut config = base_config();
        config.mode = "audio".to_string();
        config.audio = "mic".to_string();
        let args = args_for(&config);
        assert!(args.contains(&"-vn".to_string()));
        assert!(args.contains(&"-c:a".to_string()));
        assert!(args.contains(&"aac".to_string()));
        assert!(args.iter().any(|arg| arg.contains("RMS_level")));
        assert!(!args.contains(&"-vcodec".to_string()));
        assert!(!args.contains(&"scale=1920:1080".to_string()));
    }

    #[test]
    fn test_both_sources_are_mixed_and_metered() {
        let mut config = base_config();
        config.audio = "both".to_string();
        let args = args_for(&config);
        let graph = args
            .iter()
            .find(|arg| arg.contains("amix=inputs=2"))
            .expect("both sources must be mixed");
        assert!(graph.contains("[0:a][1:a]"));
        assert!(graph.contains("RMS_level"));
        assert!(args.contains(&"[aout]".to_string()));
        assert!(args.contains(&"2:v".to_string()));
    }

    #[test]
    fn test_mixed_audio_keeps_video_filter_outside_complex_graph() {
        // Mixing video into the complex graph starved the capture, so the video
        // filter must stay on `-vf` while only the audio streams are mixed.
        let mut config = base_config();
        config.audio = "both".to_string();
        config.resolution = "720p".to_string();
        let args = args_for(&config);
        let graph = args
            .iter()
            .find(|arg| arg.contains("amix=inputs=2"))
            .expect("both sources must be mixed");
        assert!(!graph.contains(":v"));
        assert!(args.iter().any(|arg| arg.contains("scale=1280:720")));
        assert!(args.contains(&"-vf".to_string()));
        assert!(!args.contains(&"[vout]".to_string()));
    }

    #[test]
    fn test_audio_only_requires_audio_source() {
        let mut config = base_config();
        config.mode = "audio".to_string();
        config.audio = "none".to_string();
        let result = RecorderEngine::build_args_with_devices(
            &config,
            Path::new("/tmp/x.m4a.tmp"),
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_single_audio_source_is_mapped_explicitly() {
        let mut config = base_config();
        config.audio = "system".to_string();
        let args = args_for(&config);
        assert!(args.contains(&"-af".to_string()));
        assert!(args.contains(&"0:a".to_string()));
        assert!(args.contains(&"1:v".to_string()));
    }

    #[test]
    fn test_quality_bitrate_ordering() {
        let low = quality_bitrate_mbps("1080p", 60, "low");
        let high = quality_bitrate_mbps("1080p", 60, "high");
        let ultra = quality_bitrate_mbps("1080p", 60, "ultra");
        assert!(low < high && high < ultra);

        let hd = quality_bitrate_mbps("720p", 60, "high");
        let quad = quality_bitrate_mbps("1440p", 60, "high");
        assert!(hd < high && high < quad);

        let thirty = quality_bitrate_mbps("1080p", 30, "high");
        let sixty = quality_bitrate_mbps("1080p", 60, "high");
        assert!(thirty < sixty);
    }

    #[test]
    fn test_audio_source_parsing() {
        assert_eq!(audio_sources("none"), (false, false));
        assert_eq!(audio_sources("mic"), (true, false));
        assert_eq!(audio_sources("system"), (false, true));
        assert_eq!(audio_sources("loopback"), (false, true));
        assert_eq!(audio_sources("both"), (true, true));
        assert!(!is_audio_enabled("none"));
        assert!(is_audio_enabled("both"));
    }

    #[test]
    fn test_unknown_mode_is_rejected() {
        let mut config = base_config();
        config.mode = "banana".to_string();
        let result = RecorderEngine::build_args_with_devices(
            &config,
            Path::new("/tmp/x.mp4.tmp"),
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_encoder_load_detection() {
        // Healthy: real time or faster.
        assert!(!encoder_is_struggling(1.01));
        assert!(!encoder_is_struggling(1.0));
        assert!(!encoder_is_struggling(1.42));
        // Encoding lag: the OBS-style dropped-frames symptom.
        assert!(encoder_is_struggling(0.4));
        assert!(encoder_is_struggling(0.84));
        // A realtime source alone keeps `speed` slightly below 1.0, so a healthy
        // run must not trip the warning.
        assert!(!encoder_is_struggling(0.89));
        assert!(!encoder_is_struggling(0.93));
        // Edge cases must not raise false alarms.
        assert!(!encoder_is_struggling(0.0));
        assert!(!encoder_is_struggling(f32::NAN));
        assert!(!encoder_is_struggling(f32::INFINITY));
    }

    #[test]
    fn test_capture_starvation_detection() {
        // Capture delivers far fewer frames than requested: the file looks like a
        // slideshow even though the encoder itself is fast.
        assert!(capture_is_starving(17.0, 60.0));
        assert!(capture_is_starving(0.0, 60.0));
        assert!(!capture_is_starving(45.0, 60.0));
        assert!(!capture_is_starving(60.0, 60.0));
        assert!(!capture_is_starving(f32::NAN, 60.0));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an interactive Windows desktop with a microphone"]
    fn test_capture_preview_returns_jpeg() {
        // Smoke test for the exact code path used by the Record tab preview.
        let frame = capture_preview("screen:0", false, 480, "auto")
            .expect("preview capture should return a frame on a desktop session");
        assert!(frame.len() > 1000, "preview frame looks empty");
        assert_eq!(&frame[..2], &[0xFF, 0xD8], "preview is not a JPEG");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an interactive Windows desktop with a microphone"]
    fn test_idle_meter_streams_rms_levels() {
        // The Record tab shows a live level before recording starts; verify the
        // exact ffmpeg invocation used by `start_idle_meter` produces RMS samples.
        use std::io::{BufRead, BufReader};

        let device = resolve_mic_device("").expect("no microphone available");
        let mut cmd = Command::new(ProcessSupervisor::resolve_ffmpeg_binary());
        cmd.args(idle_meter_args(&device))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("failed to start the meter");
        let stderr = child.stderr.take().expect("missing stderr");
        let mut lines = BufReader::new(stderr).lines();

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut samples = 0;
        while Instant::now() < deadline && samples < 3 {
            match lines.next() {
                Some(Ok(line)) if line.contains("RMS_level=") => samples += 1,
                Some(Ok(_)) => {}
                _ => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(samples >= 3, "meter produced only {samples} RMS samples");
    }
}
