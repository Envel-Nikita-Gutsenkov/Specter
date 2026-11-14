use std::fs::{self, File};
use std::io::{BufWriter, Result as IoResult};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::ring_buffer::RingBuffer;
use anyhow::Context;

pub struct StorageManager;

impl StorageManager {
    pub fn resolve_output_folder(output_folder: &str) -> PathBuf {
        let trimmed = output_folder.trim();
        if !trimmed.is_empty() {
            PathBuf::from(trimmed)
        } else {
            #[cfg(windows)]
            {
                if let Ok(user_profile) = std::env::var("USERPROFILE") {
                    let p = PathBuf::from(user_profile).join("Videos");
                    if p.exists() {
                        return p;
                    }
                }
            }
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        }
    }

    pub fn finalize_temp_file(temp_path: &Path, final_path: &Path) -> anyhow::Result<()> {
        if Self::remux(temp_path, final_path, true) {
            let _ = fs::remove_file(temp_path);
            Ok(())
        } else {
            fs::rename(temp_path, final_path).context("Failed to finalize video file")
        }
    }

    /// Remuxes a capture container into the final MP4 without re-encoding.
    /// `keep_audio = false` drops the track entirely (used for silent recordings).
    fn remux(input: &Path, output: &Path, keep_audio: bool) -> bool {
        let mut args = vec![
            "-y",
            "-fflags",
            "+genpts",
            "-i",
            input.to_str().unwrap_or_default(),
            "-c",
            "copy",
        ];
        if !keep_audio {
            args.push("-an");
        }
        args.extend([
            "-avoid_negative_ts",
            "make_zero",
            "-movflags",
            "+faststart",
            output.to_str().unwrap_or_default(),
        ]);
        Self::run_ffmpeg(&args)
    }

    /// Finalizes recorded segments (MKV while capturing, for crash safety) into a
    /// playable file. Returns the path that was actually written: normally the
    /// MP4, but the Matroska fallback if the remux fails.
    pub fn finalize_recording(
        segments: &[PathBuf],
        final_path: &Path,
        keep_audio: bool,
    ) -> anyhow::Result<PathBuf> {
        match segments.len() {
            0 => anyhow::bail!("No recording segments to finalize"),
            1 => Self::finalize_segment(&segments[0], final_path, keep_audio),
            _ => Self::concat_segments(segments, final_path, keep_audio),
        }
    }

    fn finalize_segment(
        temp_path: &Path,
        final_path: &Path,
        keep_audio: bool,
    ) -> anyhow::Result<PathBuf> {
        if Self::remux(temp_path, final_path, keep_audio) {
            let _ = fs::remove_file(temp_path);
            return Ok(final_path.to_path_buf());
        }

        // MP4 remux failed: keep the crash-safe Matroska file instead of producing
        // an unplayable result.
        let fallback = final_path.with_extension("mkv");
        let _ = fs::remove_file(&fallback);
        fs::rename(temp_path, &fallback).context("Failed to finalize recording")?;
        Ok(fallback)
    }

    fn concat_segments(
        segments: &[PathBuf],
        final_path: &Path,
        keep_audio: bool,
    ) -> anyhow::Result<PathBuf> {
        let list_path = final_path.with_extension("concat.txt");
        let mut list = String::new();
        for segment in segments {
            // The concat demuxer needs single quotes escaped like the shell does.
            let escaped = segment.to_string_lossy().replace('\'', "'\\''");
            list.push_str(&format!("file '{}'\n", escaped));
        }
        fs::write(&list_path, list).context("Failed to write concat list")?;

        let list_arg = list_path.to_str().unwrap_or_default();
        let mut remux_args = vec![
            "-y",
            "-fflags",
            "+genpts",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            list_arg,
            "-c",
            "copy",
        ];
        if !keep_audio {
            remux_args.push("-an");
        }
        remux_args.extend([
            "-avoid_negative_ts",
            "make_zero",
            "-movflags",
            "+faststart",
            final_path.to_str().unwrap_or_default(),
        ]);
        let remux_ok = Self::run_ffmpeg(&remux_args);

        if remux_ok && final_path.exists() {
            let _ = fs::remove_file(&list_path);
            for segment in segments {
                let _ = fs::remove_file(segment);
            }
            return Ok(final_path.to_path_buf());
        }

        // The MP4 muxer may reject the stream layout; try a Matroska output, which
        // accepts anything concat can produce.
        let fallback = final_path.with_extension("mkv");
        let _ = fs::remove_file(&fallback);
        let mut mkv_args = vec![
            "-y",
            "-fflags",
            "+genpts",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            list_arg,
            "-c",
            "copy",
        ];
        if !keep_audio {
            mkv_args.push("-an");
        }
        mkv_args.push(fallback.to_str().unwrap_or_default());
        let mkv_ok = Self::run_ffmpeg(&mkv_args);
        let _ = fs::remove_file(&list_path);

        if mkv_ok && fallback.exists() {
            for segment in segments {
                let _ = fs::remove_file(segment);
            }
            return Ok(fallback);
        }

        // Last resort: keep the first segment as the result and leave the rest on
        // disk untouched so a failed concat never destroys captured footage.
        let _ = fs::remove_file(&fallback);
        fs::rename(&segments[0], &fallback).context("Failed to finalize recording")?;
        Ok(fallback)
    }

    fn run_ffmpeg(args: &[&str]) -> bool {
        #[cfg(windows)]
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let ffmpeg_bin = crate::supervisor::ProcessSupervisor::resolve_ffmpeg_binary();
        let mut cmd = std::process::Command::new(&ffmpeg_bin);
        cmd.args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd.status().map(|s| s.success()).unwrap_or(false)
    }

    pub fn save_atomic(buffer: &RingBuffer, output_folder: &str) -> anyhow::Result<String> {
        let folder = Self::resolve_output_folder(output_folder);
        fs::create_dir_all(&folder).context("Failed to create destination directory")?;

        let timestamp = Self::format_timestamp();
        let final_filename = format!("Specter_{}.mp4", timestamp);
        let final_path = folder.join(&final_filename);
        let temp_path = folder.join(format!(".Specter_{}.mp4.tmp", timestamp));

        Self::write_buffer_to_file(buffer, &temp_path)?;
        Self::finalize_temp_file(&temp_path, &final_path)?;

        Ok(final_path.to_string_lossy().to_string())
    }

    pub fn write_buffer_to_file(buffer: &RingBuffer, path: &Path) -> IoResult<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        buffer.save_to_writer(&mut writer)?;
        Ok(())
    }

    pub fn format_timestamp() -> String {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();

        let s = secs % 60;
        let total_min = secs / 60;
        let mi = total_min % 60;
        let total_h = total_min / 60;
        let h = total_h % 24;
        let total_days = total_h / 24;

        let (y, mo, d) = Self::days_to_ymd(total_days);
        format!("{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", y, mo, d, h, mi, s)
    }

    fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
        days += 719468;
        let era = days / 146097;
        let doe = days % 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let mo = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if mo <= 2 { y + 1 } else { y };
        (y, mo, d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_file_saving() {
        let dir = std::env::temp_dir().join("specter_test_save");
        let mut buf = RingBuffer::new(5, 5);
        buf.init_header = b"HDR".to_vec();
        buf.assembler.init_header_ready = true;
        buf.push(b"_DATA", 1.0);

        let saved = StorageManager::save_atomic(&buf, dir.to_str().unwrap()).unwrap();
        let path = PathBuf::from(&saved);

        assert!(path.exists());
        let content = fs::read(&path).unwrap();
        assert_eq!(content, b"HDR_DATA");

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(dir);
    }

    #[test]
    fn test_finalize_single_segment_produces_output() {
        let dir = std::env::temp_dir().join("specter_test_seg_single");
        let _ = fs::create_dir_all(&dir);
        let segment = dir.join(".Specter_a_0.mp4.tmp");
        fs::write(&segment, b"DATA").unwrap();
        let final_path = dir.join("Specter_a.mp4");

        StorageManager::finalize_recording(&[segment.clone()], &final_path, true).unwrap();

        // Dummy payload cannot be remuxed to MP4, so the Matroska fallback is kept.
        let fallback = final_path.with_extension("mkv");
        assert_eq!(fs::read(&fallback).unwrap(), b"DATA");
        assert!(!segment.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_finalize_multiple_segments_keeps_data_on_concat_failure() {
        let dir = std::env::temp_dir().join("specter_test_seg_multi");
        let _ = fs::create_dir_all(&dir);
        let first = dir.join(".Specter_b_0.mp4.tmp");
        let second = dir.join(".Specter_b_1.mp4.tmp");
        fs::write(&first, b"AAA").unwrap();
        fs::write(&second, b"BBB").unwrap();
        let final_path = dir.join("Specter_b.mp4");

        StorageManager::finalize_recording(&[first.clone(), second.clone()], &final_path, true)
            .unwrap();

        // Dummy payloads cannot be concatenated by FFmpeg, so the first segment is
        // kept as a Matroska file and the rest are preserved rather than deleted.
        let fallback = final_path.with_extension("mkv");
        assert_eq!(fs::read(&fallback).unwrap(), b"AAA");
        assert!(!first.exists());
        assert!(second.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
