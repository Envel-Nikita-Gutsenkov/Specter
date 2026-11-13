# Specter Settings & Configuration

Settings are serialized as JSON and stored persistently at:
```
%APPDATA%\specter\config.json
```

All fields are protected by `#[serde(default)]` in `src-tauri/src/settings.rs` to ensure forward and backward compatibility across versions.

---

## Configuration Reference

| Key | Type | Default | Options / Valid Range | Description |
|---|---|---|---|---|
| `buffer_minutes` | `number` | `5` | `1` – `30` | Maximum temporal buffer duration in minutes. |
| `ram_limit_mb` | `number` | `500` | `100` – `4000` | Hard cap on memory buffer consumption in megabytes. |
| `resolution` | `string` | `"1080p"` | `"1080p"`, `"1440p"`, `"720p"`, `"source"` | Target recording resolution. |
| `fps` | `number` | `60` | `30`, `60`, `120`, `144` | Video capture frame rate. |
| `codec` | `string` | `"h264_nvenc"` | `"h264_nvenc"`, `"hevc_nvenc"`, `"h264_amf"`, `"hevc_amf"`, `"h264_qsv"`, `"libx264"` | Video encoder backend. |
| `bitrate_mbps` | `number \| null` | `null` | `1` – `50` (or `null` for auto) | Constant / target video encoding bitrate in megabits. |
| `audio` | `string` | `"none"` | `"none"`, `"loopback"` | Audio input: `loopback` captures system (desktop) audio via WASAPI/PulseAudio. |
| `output_folder` | `string` | `""` | Any valid Windows directory | Folder where final MP4 clips are saved (defaults to `Videos/Specter`). |
| `hotkey_toggle_buffer` | `string` | `"Alt+Shift+F8"` | Keyboard combo | Global hotkey to start or stop the replay buffer engine. |
| `hotkey_save_continue` | `string` | `"F9"` | Keyboard combo | Save buffer / start continuous disk recording. When recording, stops & saves. |
| `hotkey_save_stop` | `string` | `"F10"` | Keyboard combo | Save current replay / recording and immediately terminate buffer. |
| `capture_source` | `string` | `"screen:0"` | `"screen:N"` or `"window:PID"` | Specific monitor index or target application window (by process ID). |
| `capture_cursor` | `boolean` | `false` | `true`, `false` | Capture mouse cursor on video. Keep disabled to avoid Windows DWM cursor flicker. |
| `start_minimized` | `boolean` | `false` | `true`, `false` | Launch directly to the Windows system tray. |
| `show_overlay` | `boolean` | `true` | `true`, `false` | Display always-on-top HUD widget. |

---

## Example `config.json`

```json
{
  "buffer_minutes": 5,
  "ram_limit_mb": 500,
  "resolution": "1080p",
  "fps": 60,
  "codec": "h264_nvenc",
  "bitrate_mbps": 8,
  "audio": "none",
  "output_folder": "C:\\Users\\Nikita\\Videos\\Specter",
  "hotkey_toggle_buffer": "Alt+Shift+F8",
  "hotkey_save_continue": "F9",
  "hotkey_save_stop": "F10",
  "capture_source": "screen:0",
  "capture_cursor": false,
  "start_minimized": false,
  "show_overlay": false
}
```
