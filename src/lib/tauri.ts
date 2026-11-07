import { invoke } from "@tauri-apps/api/core";
import type { Settings, BufferStats, Monitor, WindowInfo } from "./types";

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
    return { used_mb: 0, limit_mb: 500, duration_seconds: 0 };
  }
  return await invoke<BufferStats>("get_buffer_stats");
}

export async function getSettings(): Promise<Settings> {
  if (!isTauri()) {
    return {
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
      start_minimized: false,
      show_overlay: true,
    };
  }
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
