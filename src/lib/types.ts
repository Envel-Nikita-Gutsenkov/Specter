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

export type Page = "main" | "settings";
