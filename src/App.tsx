import React, { useState, useEffect, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import TitleBar from "./components/TitleBar";
import MainPage from "./pages/MainPage";
import RecordPage from "./pages/RecordPage";
import SettingsPage from "./pages/SettingsPage";
import {
  DEFAULT_SETTINGS,
  getSettings,
  getBufferStats,
  getRecordingStats,
  onRecorderStatus,
  onRecorderWarning,
} from "./lib/tauri";
import type { Settings, BufferStats, RecorderStats } from "./lib/types";

type Page = "main" | "record" | "settings";

const DEFAULT_STATS: BufferStats = {
  used_mb: 0,
  limit_mb: 500,
  duration_seconds: 0,
  is_recording_to_disk: false,
};

const DEFAULT_RECORDER_STATS: RecorderStats = {
  is_recording: false,
  duration_seconds: 0,
  size_mb: 0,
  level: 0,
  mode: "",
};

export default function App() {
  const [page, setPage] = useState<Page>("main");
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [settingsLoaded, setSettingsLoaded] = useState(false);
  const [stats, setStats] = useState<BufferStats>(DEFAULT_STATS);
  const [isBuffering, setIsBuffering] = useState(false);
  const [isRecording, setIsRecording] = useState(false);
  const [recorderWarning, setRecorderWarning] = useState<string | null>(null);
  const [recorderStats, setRecorderStats] = useState<RecorderStats>(DEFAULT_RECORDER_STATS);

  useEffect(() => {
    getSettings()
      .then((loaded) => {
        setSettings(loaded);
        setSettingsLoaded(true);
      })
      .catch(() => setSettingsLoaded(true));
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
    const unlistenRecorder = onRecorderStatus((recording) => {
      setIsRecording(recording);
      if (!recording) {
        setRecorderStats(DEFAULT_RECORDER_STATS);
        setRecorderWarning(null);
      } else {
        getRecordingStats().then(setRecorderStats).catch(() => {});
      }
    });
    const unlistenWarning = onRecorderWarning((message) =>
      setRecorderWarning(message ? message : null)
    );
    return () => {
      unlistenStatus.then((fn) => fn()).catch(() => {});
      unlistenClip.then((fn) => fn()).catch(() => {});
      unlistenRecorder();
      unlistenWarning();
    };
  }, [settings.ram_limit_mb]);

  useEffect(() => {
    if (!isBuffering) return;
    getBufferStats()
      .then(setStats)
      .catch(() => {});
    const id = setInterval(() => {
      getBufferStats()
        .then(setStats)
        .catch(() => {});
    }, 500);
    return () => clearInterval(id);
  }, [isBuffering]);

  useEffect(() => {
    if (!isRecording) return;
    getRecordingStats()
      .then(setRecorderStats)
      .catch(() => {});
    const id = setInterval(() => {
      getRecordingStats()
        .then((s) => {
          setRecorderStats(s);
          if (!s.is_recording) setIsRecording(false);
        })
        .catch(() => {});
    }, 500);
    return () => clearInterval(id);
  }, [isRecording]);

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
        isRecording={isRecording}
        minimizeToTray={settings.minimize_to_tray}
      />
      <div className={`page-container ${page === "main" ? "no-scroll" : ""}`}>
        {page === "main" && (
          <MainPage
            settings={settings}
            stats={stats}
            isBuffering={isBuffering}
            onBufferingChange={setIsBuffering}
            onStatsChange={setStats}
          />
        )}
        {page === "record" && (
          <RecordPage
            settings={settings}
            settingsLoaded={settingsLoaded}
            isRecording={isRecording}
            stats={recorderStats}
            warning={recorderWarning}
            onRecordingChange={setIsRecording}
            onStatsChange={setRecorderStats}
            onSettingsChange={setSettings}
          />
        )}
        {page === "settings" && (
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
