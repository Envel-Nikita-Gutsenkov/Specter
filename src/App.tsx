import React, { useState, useEffect, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import TitleBar from "./components/TitleBar";
import MainPage from "./pages/MainPage";
import SettingsPage from "./pages/SettingsPage";
import { getSettings, getBufferStats } from "./lib/tauri";
import type { Settings, BufferStats } from "./lib/types";

type Page = "main" | "settings";

const DEFAULT_SETTINGS: Settings = {
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

const DEFAULT_STATS: BufferStats = {
  used_mb: 0,
  limit_mb: 500,
  duration_seconds: 0,
  is_recording_to_disk: false,
};

export default function App() {
  const [page, setPage] = useState<Page>("main");
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [stats, setStats] = useState<BufferStats>(DEFAULT_STATS);
  const [isBuffering, setIsBuffering] = useState(false);

  useEffect(() => {
    getSettings()
      .then(setSettings)
      .catch(() => {});
  }, []);

  useEffect(() => {
    const unlistenStatus = listen<boolean>("buffering-status", (event) => {
      setIsBuffering(event.payload);
      if (!event.payload) {
        setStats({ used_mb: 0, limit_mb: settings.ram_limit_mb, duration_seconds: 0, is_recording_to_disk: false });
      }
    });
    const unlistenClip = listen<string>("clip-saved", () => {
      getBufferStats().then(setStats).catch(() => {});
    });
    return () => {
      unlistenStatus.then((fn) => fn()).catch(() => {});
      unlistenClip.then((fn) => fn()).catch(() => {});
    };
  }, [settings.ram_limit_mb]);

  useEffect(() => {
    if (!isBuffering) return;
    const id = setInterval(() => {
      getBufferStats()
        .then(setStats)
        .catch(() => {});
    }, 500);
    return () => clearInterval(id);
  }, [isBuffering]);

  useEffect(() => {
    const handleGlobalKeyDown = (e: KeyboardEvent) => {
      if (e.code === "Space") {
        const target = e.target as HTMLElement | null;
        const isInput =
          target &&
          (target.tagName === "INPUT" ||
            target.tagName === "TEXTAREA" ||
            target.isContentEditable);
        if (!isInput) {
          e.preventDefault();
          e.stopPropagation();
        }
      }
    };
    window.addEventListener("keydown", handleGlobalKeyDown, true);
    return () => window.removeEventListener("keydown", handleGlobalKeyDown, true);
  }, []);

  const handleSettingsSave = useCallback((s: Settings) => {
    setSettings(s);
    setPage("main");
  }, []);

  return (
    <div className="app-root">
      <TitleBar
        activePage={page}
        onNavigate={setPage}
        isBuffering={isBuffering}
      />
      <div className={`page-container ${page === "main" ? "no-scroll" : ""}`}>
        {page === "main" ? (
          <MainPage
            settings={settings}
            stats={stats}
            isBuffering={isBuffering}
            onBufferingChange={setIsBuffering}
            onStatsChange={setStats}
          />
        ) : (
          <SettingsPage
            settings={settings}
            onSave={handleSettingsSave}
            onCancel={() => setPage("main")}
          />
        )}
      </div>
    </div>
  );
}
