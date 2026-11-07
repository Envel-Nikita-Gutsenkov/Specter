use std::fs::{self, File};
use std::io::{BufWriter, Result as IoResult};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use crate::ring_buffer::RingBuffer;

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
        #[cfg(windows)]
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let ffmpeg_bin = crate::supervisor::ProcessSupervisor::resolve_ffmpeg_binary();
        let mut cmd = std::process::Command::new(&ffmpeg_bin);
        cmd.args([
            "-y",
            "-i", temp_path.to_str().unwrap_or_default(),
            "-c", "copy",
            "-movflags", "+faststart",
            final_path.to_str().unwrap_or_default(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let remux_ok = cmd.status().map(|s| s.success()).unwrap_or(false);

        if remux_ok {
            let _ = fs::remove_file(temp_path);
        } else {
            fs::rename(temp_path, final_path).context("Failed to finalize video file")?;
        }

        Ok(())
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
}
