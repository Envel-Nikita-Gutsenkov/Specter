export interface Settings {
  buffer_minutes: number;
  ram_limit_mb: number;
  resolution: string;
  fps: number;
  codec: string;
  bitrate_mbps: number | null;
  audio: string;
  output_folder: string;
  hotkey_toggle_buffer: string;
  hotkey_save_continue: string;
  hotkey_save_stop: string;
  capture_source: string;
  capture_cursor: boolean;
  start_minimized: boolean;
  show_overlay: boolean;
  minimize_to_tray: boolean;
  recording_mode: string;
  recording_resolution: string;
  recording_fps: number;
  recording_audio: string;
  recording_quality: string;
  recording_codec: string;
  mic_device: string;
  capture_backend: string;
  video_preset: string;
}

export interface BufferStats {
  used_mb: number;
  limit_mb: number;
  duration_seconds: number;
  is_recording_to_disk: boolean;
}

export interface Monitor {
  id: string;
  name: string;
  resolution: string;
}

export interface WindowInfo {
  pid: number;
  title: string;
  exe: string;
}

export type RecordingMode = "video" | "audio";
export type RecordingQuality = "low" | "medium" | "high" | "ultra";
export type RecordingAudio = "none" | "mic" | "system" | "both";

export interface RecordingConfig {
  mode: RecordingMode;
  resolution: string;
  fps: number;
  quality: RecordingQuality;
  audio: RecordingAudio;
  mic_device: string;
  codec: string;
  capture_source: string;
  capture_cursor: boolean;
  capture_backend: string;
  video_preset: string;
  output_folder: string;
}

export interface RecorderStats {
  is_recording: boolean;
  duration_seconds: number;
  size_mb: number;
  level: number;
  mode: string;
}

export type Page = "main" | "record" | "settings";
