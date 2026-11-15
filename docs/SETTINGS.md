# Specter Settings & Configuration

Settings are serialized as JSON and stored at:

```
%APPDATA%\specter\config.json
```

Every field uses `#[serde(default)]` (`src-tauri/src/settings.rs`), so a config written by
one version still loads in another: missing keys fall back to their defaults.

---

## Configuration Reference

| Key | Type | Default | Options / Valid Range | Description |
|---|---|---|---|---|
| `buffer_minutes` | `number` | `5` | `1` – `60` | Maximum duration kept in the replay buffer. |
| `ram_limit_mb` | `number` | `500` | `100` – `2000` | Hard cap on memory used by the buffer. |
| `resolution` | `string` | `"1080p"` | `"native"`, `"1440p"`, `"1080p"`, `"720p"` | Buffer output resolution; `native` keeps the capture size. |
| `fps` | `number` | `60` | `30`, `60` | Buffer frame rate. |
| `codec` | `string` | `"h264_nvenc"` | Settings UI: `"h264_nvenc"`, `"hevc_nvenc"`, `"libx264"`. Editable in the file: `"h264_amf"`, `"hevc_amf"`, `"h264_qsv"`, `"hevc_qsv"`, `"libx265"` | Encoder backend. |
| `bitrate_mbps` | `number \| null` | `null` | `1` – `100`, or `null` | Target video bitrate; `null` derives it from resolution, FPS and quality preset. |
| `audio` | `string` | `"none"` | `"none"`, `"loopback"` | Buffer audio. `loopback` records the system output through WASAPI. |
| `output_folder` | `string` | `<Videos>\Specter` | Any directory | Where final clips are saved. Resolved from the user's Videos folder on first launch. |
| `hotkey_toggle_buffer` | `string` | `"Alt+Shift+F8"` | Keyboard combo | Start or stop the replay buffer. |
| `hotkey_save_continue` | `string` | `"F9"` | Keyboard combo | Flush the buffer and keep recording to disk. While recording: stop and save. |
| `hotkey_save_stop` | `string` | `"F10"` | Keyboard combo | Save and stop the capture engine. |
| `capture_source` | `string` | `"screen:0"` | `"screen:N"` or `"window:PID"` | Monitor index or target process id. |
| `capture_cursor` | `boolean` | `false` | `true`, `false` | Draw the mouse cursor into the capture. |
| `start_minimized` | `boolean` | `false` | `true`, `false` | Start hidden in the system tray. |
| `show_overlay` | `boolean` | `true` | `true`, `false` | Show the always-on-top status overlay. |
| `minimize_to_tray` | `boolean` | `false` | `true`, `false` | Minimize to the tray instead of the taskbar. |
| `recording_mode` | `string` | `"video"` | `"video"`, `"audio"` | Record tab mode: video (MP4) or audio only (M4A). |
| `recording_resolution` | `string` | `"1080p"` | `"source"`, `"720p"`, `"1080p"`, `"1440p"` | Record tab output resolution; `source` keeps the native size. |
| `recording_fps` | `number` | `60` | `30`, `48`, `60`, `120`, `144` | Record tab frame rate. |
| `recording_audio` | `string` | `"none"` | `"none"`, `"mic"`, `"system"`, `"both"` | Record tab audio; `both` mixes microphone and system audio with `amix`. |
| `recording_quality` | `string` | `"high"` | `"low"`, `"medium"`, `"high"`, `"ultra"` | Preset for video bitrate and AAC bitrate (160 / 192 / 256 / 320 kbps). |
| `recording_codec` | `string` | `"h264_nvenc"` | `"h264_nvenc"`, `"hevc_nvenc"`, `"libx264"`, `"libx265"` | Record tab encoder; also accepts the AMF/QSV names listed under `codec`. |
| `mic_device` | `string` | `""` | DirectShow device name | Preferred microphone. Empty selects the first detected device. |
| `capture_backend` | `string` | `"auto"` | `"auto"`, `"dda"`, `"gdi"` | Capture path for buffers and recordings. `auto`/`dda` use Desktop Duplication (`ddagrab`) for monitors, `gdi` forces `gdigrab`. Window capture always uses `gdigrab`. |
| `video_preset` | `string` | `"balanced"` | `"speed"`, `"balanced"`, `"quality"` | Encoder speed/quality trade-off, mapped per encoder family (NVENC `p1`/`p4`/`p6`, AMF, QSV, libx264/libx265). |

Ranges are also enforced by the UI controls in `src/pages/SettingsPage.tsx`; values outside
them can still be written to the file directly.

---

## Example `config.json`

```json
{
  "buffer_minutes": 5,
  "ram_limit_mb": 500,
  "resolution": "1080p",
  "fps": 60,
  "codec": "h264_nvenc",
  "bitrate_mbps": null,
  "audio": "none",
  "output_folder": "C:\\Users\\you\\Videos\\Specter",
  "hotkey_toggle_buffer": "Alt+Shift+F8",
  "hotkey_save_continue": "F9",
  "hotkey_save_stop": "F10",
  "capture_source": "screen:0",
  "capture_cursor": false,
  "start_minimized": false,
  "show_overlay": true,
  "minimize_to_tray": false,
  "recording_mode": "video",
  "recording_resolution": "1080p",
  "recording_fps": 60,
  "recording_audio": "none",
  "recording_quality": "high",
  "recording_codec": "h264_nvenc",
  "mic_device": "",
  "capture_backend": "auto",
  "video_preset": "balanced"
}
```
