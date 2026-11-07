import React, { useState, useEffect } from "react";
import HotkeyInput from "../components/HotkeyInput";
import type { Settings, Monitor, WindowInfo } from "../lib/types";
import { saveSettings, getMonitors, getWindows } from "../lib/tauri";
import { open } from "@tauri-apps/plugin-dialog";

interface Props {
  settings: Settings;
  onSave: (s: Settings) => void;
  onCancel: () => void;
}

function SettingRow({ label, children, hint }: { label: string; children: React.ReactNode; hint?: string }) {
  return (
    <div className="setting-row">
      <div className="setting-row-label">
        <label>{label}</label>
        {hint && <span className="setting-hint">{hint}</span>}
      </div>
      <div className="setting-row-control">{children}</div>
    </div>
  );
}

function SettingsGroup({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="settings-group">
      <div className="section-title">{title}</div>
      <div className="settings-group-body">{children}</div>
    </div>
  );
}

export default function SettingsPage({ settings, onSave, onCancel }: Props) {
  const [draft, setDraft] = useState<Settings>({ ...settings });
  const [monitors, setMonitors] = useState<Monitor[]>([]);
  const [windows, setWindows] = useState<WindowInfo[]>([]);
  const [saving, setSaving] = useState(false);
  const [customMinutes, setCustomMinutes] = useState<string>(
    String(settings.buffer_minutes)
  );
  const [bufferMode, setBufferMode] = useState<"5" | "10" | "custom">(
    settings.buffer_minutes === 5 ? "5" : settings.buffer_minutes === 10 ? "10" : "custom"
  );

  useEffect(() => {
    getMonitors().then(setMonitors).catch(() => {});
    getWindows().then(setWindows).catch(() => {});
  }, []);

  function update<K extends keyof Settings>(key: K, value: Settings[K]) {
    setDraft((prev) => ({ ...prev, [key]: value }));
  }

  async function handlePickFolder() {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (selected && typeof selected === "string") {
        update("output_folder", selected);
      }
    } catch (_) {}
  }

  async function handleSave() {
    setSaving(true);
    const finalDraft = {
      ...draft,
      buffer_minutes:
        bufferMode === "custom"
          ? Math.min(60, Math.max(1, parseInt(customMinutes) || 5))
          : parseInt(bufferMode),
    };
    try {
      await saveSettings(finalDraft);
    } catch (_) {}
    setSaving(false);
    onSave(finalDraft);
  }

  const isCaptureScreen = draft.capture_source.startsWith("screen:");

  return (
    <div className="settings-page fade-in">
      <SettingsGroup title="Capture Source">
        <SettingRow label="Source Type">
          <div className="seg-ctrl">
            <button
              className={`seg-btn ${isCaptureScreen ? "seg-btn--active" : ""}`}
              onClick={() => update("capture_source", "screen:0")}
            >
              Monitor
            </button>
            <button
              className={`seg-btn ${!isCaptureScreen ? "seg-btn--active" : ""}`}
              onClick={() => {
                const first = windows[0];
                update("capture_source", first ? `window:${first.pid}` : "window:0");
              }}
            >
              Window
            </button>
          </div>
        </SettingRow>

        {isCaptureScreen ? (
          <SettingRow label="Monitor">
            <select
              value={draft.capture_source}
              onChange={(e) => update("capture_source", e.target.value)}
              className="full-select"
            >
              {monitors.length === 0 && (
                <option value="screen:0">Display 1 (Primary)</option>
              )}
              {monitors.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name} ({m.resolution})
                </option>
              ))}
            </select>
          </SettingRow>
        ) : (
          <SettingRow label="Window">
            <select
              value={draft.capture_source}
              onChange={(e) => update("capture_source", e.target.value)}
              className="full-select"
            >
              {windows.length === 0 && (
                <option value="window:0">No windows found</option>
              )}
              {windows.map((w) => (
                <option key={w.pid} value={`window:${w.pid}`}>
                  {w.title} ({w.exe})
                </option>
              ))}
            </select>
          </SettingRow>
        )}
      </SettingsGroup>

      <SettingsGroup title="Buffer">
        <SettingRow label="Duration">
          <div className="seg-ctrl">
            {(["5", "10", "custom"] as const).map((v) => (
              <button
                key={v}
                className={`seg-btn ${bufferMode === v ? "seg-btn--active" : ""}`}
                onClick={() => setBufferMode(v)}
              >
                {v === "5" ? "5 min" : v === "10" ? "10 min" : "Custom"}
              </button>
            ))}
          </div>
          {bufferMode === "custom" && (
            <div className="custom-min-row">
              <input
                type="number"
                min={1}
                max={60}
                value={customMinutes}
                onChange={(e) => setCustomMinutes(e.target.value)}
                className="number-input"
              />
              <span className="unit-label">minutes (max 60)</span>
            </div>
          )}
        </SettingRow>

        <SettingRow label="RAM Limit" hint={`${draft.ram_limit_mb} MB`}>
          <div className="slider-wrap">
            <span className="slider-min">100 MB</span>
            <input
              type="range"
              min={100}
              max={2000}
              step={50}
              value={draft.ram_limit_mb}
              onChange={(e) => update("ram_limit_mb", Number(e.target.value))}
              className="range-input"
            />
            <span className="slider-max">2 GB</span>
          </div>
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title="Video">
        <SettingRow label="Resolution">
          <select
            value={draft.resolution}
            onChange={(e) => update("resolution", e.target.value)}
            className="full-select"
          >
            <option value="native">Native</option>
            <option value="1440p">1440p</option>
            <option value="1080p">1080p</option>
            <option value="720p">720p</option>
          </select>
        </SettingRow>

        <SettingRow label="FPS">
          <div className="seg-ctrl">
            {([30, 60] as const).map((v) => (
              <button
                key={v}
                className={`seg-btn ${draft.fps === v ? "seg-btn--active" : ""}`}
                onClick={() => update("fps", v)}
              >
                {v}
              </button>
            ))}
          </div>
        </SettingRow>

        <SettingRow label="Codec">
          <select
            value={draft.codec}
            onChange={(e) => update("codec", e.target.value)}
            className="full-select"
          >
            <option value="h264_nvenc">H.264 NVENC (GPU)</option>
            <option value="hevc_nvenc">H.265 NVENC (GPU)</option>
            <option value="libx264">H.264 CPU Fallback</option>
          </select>
        </SettingRow>

        <SettingRow label="Bitrate">
          <div className="seg-ctrl">
            <button
              className={`seg-btn ${draft.bitrate_mbps === null ? "seg-btn--active" : ""}`}
              onClick={() => update("bitrate_mbps", null)}
            >
              Auto
            </button>
            <button
              className={`seg-btn ${draft.bitrate_mbps !== null ? "seg-btn--active" : ""}`}
              onClick={() => update("bitrate_mbps", 6)}
            >
              Manual
            </button>
          </div>
          {draft.bitrate_mbps !== null && (
            <div className="custom-min-row">
              <input
                type="number"
                min={1}
                max={100}
                step={0.5}
                value={draft.bitrate_mbps}
                onChange={(e) => update("bitrate_mbps", parseFloat(e.target.value) || 6)}
                className="number-input"
              />
              <span className="unit-label">Mbps</span>
            </div>
          )}
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title="Audio">
        <SettingRow label="Audio Source">
          <div className="seg-ctrl">
            <button
              className={`seg-btn ${draft.audio === "none" ? "seg-btn--active" : ""}`}
              onClick={() => update("audio", "none")}
            >
              None
            </button>
            <button
              className={`seg-btn ${draft.audio === "loopback" ? "seg-btn--active" : ""}`}
              onClick={() => update("audio", "loopback")}
            >
              System Audio
            </button>
          </div>
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title="Output">
        <SettingRow label="Output Folder">
          <div className="folder-row">
            <input
              type="text"
              value={draft.output_folder || "%USERPROFILE%\\Videos\\Specter"}
              onChange={(e) => update("output_folder", e.target.value)}
              className="text-input"
              placeholder="%USERPROFILE%\Videos\Specter"
            />
            <button className="pill-btn pill-btn-ghost folder-browse-btn" onClick={handlePickFolder}>
              Browse
            </button>
          </div>
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title="Hotkeys">
        <SettingRow label="Toggle Buffering">
          <HotkeyInput
            value={draft.hotkey_toggle_buffer || "Alt+Shift+F8"}
            onChange={(k) => update("hotkey_toggle_buffer", k)}
          />
        </SettingRow>
        <SettingRow label="Save + Continue">
          <HotkeyInput
            value={draft.hotkey_save_continue}
            onChange={(k) => update("hotkey_save_continue", k)}
          />
        </SettingRow>
        <SettingRow label="Save + Stop">
          <HotkeyInput
            value={draft.hotkey_save_stop}
            onChange={(k) => update("hotkey_save_stop", k)}
          />
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title="App">
        <SettingRow label="Start Minimized to Tray">
          <button
            className={`toggle-switch ${draft.start_minimized ? "toggle-switch--on" : ""}`}
            onClick={() => update("start_minimized", !draft.start_minimized)}
            role="switch"
            aria-checked={draft.start_minimized}
          >
            <span className="toggle-thumb" />
          </button>
        </SettingRow>
        <SettingRow label="Show Overlay Widget">
          <button
            className={`toggle-switch ${draft.show_overlay ? "toggle-switch--on" : ""}`}
            onClick={() => update("show_overlay", !draft.show_overlay)}
            role="switch"
            aria-checked={draft.show_overlay}
          >
            <span className="toggle-thumb" />
          </button>
        </SettingRow>
      </SettingsGroup>

      <div className="settings-actions">
        <button className="pill-btn pill-btn-ghost" onClick={onCancel}>
          Cancel
        </button>
        <button className="pill-btn pill-btn-primary" onClick={handleSave} disabled={saving}>
          {saving ? "Saving…" : "Save Settings"}
        </button>
      </div>

      <style>{`
        .settings-page {
          padding: 20px;
          display: flex;
          flex-direction: column;
          gap: 20px;
        }
        .settings-group {
          display: flex;
          flex-direction: column;
          gap: 0;
        }
        .settings-group-body {
          background: var(--surface);
          border: 1px solid var(--border);
          border-radius: var(--radius);
          overflow: hidden;
        }
        .setting-row {
          display: flex;
          align-items: center;
          justify-content: space-between;
          gap: 12px;
          padding: 12px 14px;
          border-bottom: 1px solid var(--border);
          flex-wrap: wrap;
        }
        .setting-row:last-child {
          border-bottom: none;
        }
        .setting-row-label {
          display: flex;
          flex-direction: column;
          gap: 2px;
          flex-shrink: 0;
        }
        .setting-hint {
          font-size: 11px;
          color: var(--accent);
          font-weight: 600;
          font-variant-numeric: tabular-nums;
        }
        .setting-row-control {
          display: flex;
          flex-direction: column;
          gap: 6px;
          min-width: 160px;
          flex: 1;
          align-items: flex-end;
        }
        .seg-ctrl {
          display: flex;
          background: var(--surface-2);
          border: 1px solid var(--border);
          border-radius: var(--radius-pill);
          padding: 3px;
          gap: 2px;
        }
        .seg-btn {
          padding: 4px 12px;
          border-radius: var(--radius-pill);
          font-size: 12px;
          font-weight: 500;
          color: var(--text-dim);
          background: transparent;
          transition: all var(--transition);
          white-space: nowrap;
        }
        .seg-btn:hover {
          color: var(--text-muted);
        }
        .seg-btn--active {
          background: var(--surface-3);
          color: var(--text);
        }
        .full-select {
          width: 100%;
          padding: 7px 10px;
          border-radius: var(--radius-sm);
          font-size: 12px;
          cursor: pointer;
        }
        .slider-wrap {
          display: flex;
          align-items: center;
          gap: 8px;
          width: 100%;
        }
        .slider-min, .slider-max {
          font-size: 10px;
          color: var(--text-dim);
          white-space: nowrap;
        }
        .range-input {
          flex: 1;
          height: 4px;
          appearance: none;
          background: var(--surface-3);
          border-radius: 2px;
          border: none;
          outline: none;
          cursor: pointer;
          accent-color: var(--accent);
        }
        .range-input::-webkit-slider-thumb {
          appearance: none;
          width: 16px;
          height: 16px;
          border-radius: 50%;
          background: var(--accent);
          box-shadow: 0 0 8px var(--accent-glow);
          cursor: pointer;
          transition: box-shadow var(--transition);
        }
        .range-input::-webkit-slider-thumb:hover {
          box-shadow: 0 0 14px var(--accent-glow);
        }
        .custom-min-row {
          display: flex;
          align-items: center;
          gap: 8px;
          width: 100%;
          justify-content: flex-end;
        }
        .number-input {
          width: 70px;
          padding: 5px 8px;
          text-align: center;
          border-radius: var(--radius-sm);
        }
        .unit-label {
          font-size: 11px;
          color: var(--text-dim);
        }
        .folder-row {
          display: flex;
          gap: 8px;
          width: 100%;
        }
        .text-input {
          flex: 1;
          padding: 7px 10px;
          border-radius: var(--radius-sm);
          font-size: 12px;
          background: var(--surface-2);
          border: 1px solid var(--border);
        }
        .folder-browse-btn {
          flex-shrink: 0;
          font-size: 12px;
          padding: 7px 14px;
        }
        .toggle-switch {
          position: relative;
          width: 40px;
          height: 22px;
          border-radius: var(--radius-pill);
          background: var(--surface-3);
          border: 1px solid var(--border);
          transition: background 0.25s ease, border-color 0.25s ease;
          flex-shrink: 0;
        }
        .toggle-switch--on {
          background: var(--accent);
          border-color: var(--accent);
          box-shadow: 0 0 10px var(--accent-glow);
        }
        .toggle-thumb {
          position: absolute;
          top: 2px;
          left: 2px;
          width: 16px;
          height: 16px;
          border-radius: 50%;
          background: #fff;
          transition: transform 0.25s ease;
          box-shadow: 0 1px 4px rgba(0,0,0,0.3);
        }
        .toggle-switch--on .toggle-thumb {
          transform: translateX(18px);
        }
        .settings-actions {
          display: flex;
          gap: 10px;
          justify-content: flex-end;
          padding-bottom: 4px;
        }
      `}</style>
    </div>
  );
}
