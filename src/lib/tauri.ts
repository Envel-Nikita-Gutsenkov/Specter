import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  Settings,
  BufferStats,
  Monitor,
  WindowInfo,
  RecordingConfig,
  RecorderStats,
} from "./types";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function startBuffer(settings: Settings): Promise<void> {
  if (!isTauri()) return;
  await invoke<void>("start_buffer", { settings });
}

export async function stopBuffer(): Promise<void> {
  if (!isTauri()) return;
  await invoke<void>("stop_buffer");
}

export async function saveClip(continueRecording: boolean): Promise<string> {
  if (!isTauri()) return "mock/Specter_2024-11-07_20-18-00.mp4";
  return await invoke<string>("save_clip", { continueRecording });
}

export async function getBufferStats(): Promise<BufferStats> {
  if (!isTauri()) {
    return { used_mb: 0, limit_mb: 500, duration_seconds: 0, is_recording_to_disk: false };
  }
  return await invoke<BufferStats>("get_buffer_stats");
}

export const DEFAULT_SETTINGS: Settings = {
  buffer_minutes: 5,
  ram_limit_mb: 500,
  resolution: "1080p",
  fps: 60,
  codec: "h264_nvenc",
  bitrate_mbps: null,
  audio: "none",
  output_folder: "",
  hotkey_toggle_buffer: "Alt+Shift+F8",
  hotkey_save_continue: "F9",
  hotkey_save_stop: "F10",
  capture_source: "screen:0",
  capture_cursor: false,
  start_minimized: false,
  show_overlay: true,
  minimize_to_tray: false,
  recording_mode: "video",
  recording_resolution: "1080p",
  recording_fps: 60,
  recording_audio: "none",
  recording_quality: "high",
  recording_codec: "h264_nvenc",
  mic_device: "",
  capture_backend: "auto",
  video_preset: "balanced",
};

export async function getSettings(): Promise<Settings> {
  if (!isTauri()) return { ...DEFAULT_SETTINGS };
  return await invoke<Settings>("get_settings");
}

export async function saveSettings(settings: Settings): Promise<void> {
  if (!isTauri()) return;
  await invoke<void>("save_settings", { settings });
}

export async function getMonitors(): Promise<Monitor[]> {
  if (!isTauri()) {
    return [
      { id: "screen:0", name: "Display 1 (Primary)", resolution: "1920x1080" },
      { id: "screen:1", name: "Display 2", resolution: "2560x1440" },
    ];
  }
  return await invoke<Monitor[]>("get_monitors");
}

export async function getWindows(): Promise<WindowInfo[]> {
  if (!isTauri()) {
    return [
      { pid: 1234, title: "Notepad", exe: "notepad.exe" },
      { pid: 5678, title: "Google Chrome", exe: "chrome.exe" },
    ];
  }
  return await invoke<WindowInfo[]>("get_windows");
}

// ---------------------------------------------------------------------------
// Recorder
// ---------------------------------------------------------------------------

const MOCK_VIDEO_MB_PER_SEC = 1.6;
const MOCK_AUDIO_MB_PER_SEC = 0.024;

let mockRecording = false;
let mockStartedAt = 0;
let mockMode = "video";
let mockLevelTimer: ReturnType<typeof setInterval> | null = null;
const mockLevelSubs = new Set<(level: number) => void>();
const mockStatusSubs = new Set<(recording: boolean) => void>();

function startMockMeter() {
  if (mockLevelTimer) return;
  let value = 0.2;
  mockLevelTimer = setInterval(() => {
    const target = 0.15 + Math.random() * 0.65;
    value = value * 0.55 + target * 0.45;
    mockLevelSubs.forEach((cb) => cb(value));
  }, 60);
}

function stopMockMeter() {
  if (mockLevelTimer) {
    clearInterval(mockLevelTimer);
    mockLevelTimer = null;
  }
  mockLevelSubs.forEach((cb) => cb(0));
}

function setMockRecording(recording: boolean) {
  mockRecording = recording;
  mockStatusSubs.forEach((cb) => cb(recording));
  if (recording) {
    mockStartedAt = performance.now();
    startMockMeter();
  } else {
    mockStartedAt = 0;
    stopMockMeter();
  }
}

export async function startRecording(config: RecordingConfig): Promise<void> {
  if (!isTauri()) {
    mockMode = config.mode;
    setMockRecording(true);
    return;
  }
  await invoke<void>("start_recording", { config });
}

export async function stopRecording(): Promise<string> {
  if (!isTauri()) {
    setMockRecording(false);
    return mockMode === "audio"
      ? "mock/Specter_2024-11-07_20-18-00.m4a"
      : "mock/Specter_2024-11-07_20-18-00.mp4";
  }
  return await invoke<string>("stop_recording");
}

export async function switchRecordingSource(config: RecordingConfig): Promise<void> {
  if (!isTauri()) {
    mockMode = config.mode;
    return;
  }
  await invoke<void>("switch_recording_source", { config });
}

export async function getRecordingStats(): Promise<RecorderStats> {
  if (!isTauri()) {
    if (!mockRecording) {
      return { is_recording: false, duration_seconds: 0, size_mb: 0, level: 0, mode: "" };
    }
    const duration = (performance.now() - mockStartedAt) / 1000;
    const rate = mockMode === "audio" ? MOCK_AUDIO_MB_PER_SEC : MOCK_VIDEO_MB_PER_SEC;
    return {
      is_recording: true,
      duration_seconds: duration,
      size_mb: duration * rate,
      level: 0,
      mode: mockMode,
    };
  }
  return await invoke<RecorderStats>("get_recording_stats");
}

export async function capturePreview(
  captureSource: string,
  captureCursor: boolean,
  captureBackend = "auto",
  maxWidth = 480
): Promise<Blob | null> {
  if (!isTauri()) return null;
  const data = await invoke<ArrayBuffer>("capture_preview", {
    captureSource,
    captureCursor,
    captureBackend,
    maxWidth,
  });
  if (!data || data.byteLength === 0) return null;
  return new Blob([data], { type: "image/jpeg" });
}

export async function getAudioDevices(): Promise<string[]> {
  if (!isTauri()) {
    return ["Microphone (Realtek Audio)", "Stereo Mix (Realtek Audio)"];
  }
  return await invoke<string[]>("get_audio_devices");
}

export async function hideToTray(): Promise<void> {
  if (!isTauri()) return;
  await invoke<void>("app_hide");
}

export async function startAudioMeter(audio: string, micDevice: string): Promise<void> {
  if (!isTauri()) {
    startMockMeter();
    return;
  }
  await invoke<void>("start_audio_meter", { audio, micDevice });
}

export async function stopAudioMeter(): Promise<void> {
  if (!isTauri()) {
    stopMockMeter();
    return;
  }
  await invoke<void>("stop_audio_meter");
}

export function onRecorderLevel(cb: (level: number) => void): () => void {
  if (!isTauri()) {
    mockLevelSubs.add(cb);
    return () => {
      mockLevelSubs.delete(cb);
    };
  }
  let unlisten: (() => void) | null = null;
  let cancelled = false;
  listen<number>("recorder-level", (event) => cb(event.payload))
    .then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    })
    .catch(() => {});
  return () => {
    cancelled = true;
    unlisten?.();
  };
}

export function onRecorderStatus(cb: (recording: boolean) => void): () => void {
  if (!isTauri()) {
    mockStatusSubs.add(cb);
    return () => {
      mockStatusSubs.delete(cb);
    };
  }
  let unlisten: (() => void) | null = null;
  let cancelled = false;
  listen<boolean>("recorder-status", (event) => cb(event.payload))
    .then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    })
    .catch(() => {});
  return () => {
    cancelled = true;
    unlisten?.();
  };
}

export function onRecorderWarning(cb: (message: string) => void): () => void {
  if (!isTauri()) return () => {};
  let unlisten: (() => void) | null = null;
  let cancelled = false;
  listen<string>("recorder-warning", (event) => cb(event.payload))
    .then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    })
    .catch(() => {});
  return () => {
    cancelled = true;
    unlisten?.();
  };
}

export { isTauri };
