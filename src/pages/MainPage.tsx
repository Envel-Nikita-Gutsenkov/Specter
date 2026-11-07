import React, { useState, useCallback, useMemo } from "react";
import BufferBar from "../components/BufferBar";
import type { Settings, BufferStats } from "../lib/types";
import { startBuffer, stopBuffer, saveClip } from "../lib/tauri";

interface Props {
  settings: Settings;
  stats: BufferStats;
  isBuffering: boolean;
  onBufferingChange: (v: boolean) => void;
  onStatsChange: (s: BufferStats) => void;
}

const SIZE = 200;
const STROKE = 14;
const R = (SIZE - STROKE) / 2;
const CIRCUMFERENCE = 2 * Math.PI * R;
const GAP_DEG = 70;
const ARC_DEG = 360 - GAP_DEG;
const ARC_LEN = CIRCUMFERENCE * (ARC_DEG / 360);
const START_ANGLE = 90 + GAP_DEG / 2;

function formatDuration(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = Math.floor(secs % 60);
  if (h > 0) return `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function formatMb(mb: number): string {
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  return `${Math.round(mb)} MB`;
}

export default function MainPage({
  settings,
  stats,
  isBuffering,
  onBufferingChange,
  onStatsChange,
}: Props) {
  const [toast, setToast] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [mockPct, setMockPct] = useState(0);

  const isRecording = Boolean(stats.is_recording_to_disk);

  const pct = useMemo(
    () => (stats.limit_mb > 0 ? Math.min(stats.used_mb / stats.limit_mb, 1) : 0),
    [stats]
  );

  const displayPct = isBuffering && stats.used_mb === 0 ? mockPct : pct;

  const arcColor = useMemo(() => {
    if (isRecording) return "#ef4444";
    if (displayPct > 0.9) return "#ef4444";
    if (displayPct > 0.7) return "#f59e0b";
    return "#7c3aed";
  }, [displayPct, isRecording]);

  const dashOffset = isRecording ? 0 : ARC_LEN * (1 - displayPct);

  function showToast(msg: string) {
    setToast(msg);
    setTimeout(() => setToast(null), 3000);
  }

  async function handleToggle() {
    if (isBuffering) {
      try {
        await stopBuffer();
        onBufferingChange(false);
        onStatsChange({ used_mb: 0, limit_mb: settings.ram_limit_mb, duration_seconds: 0, is_recording_to_disk: false });
        setMockPct(0);
      } catch (e) {
        showToast(`Error: ${e}`);
      }
    } else {
      try {
        await startBuffer(settings);
        onBufferingChange(true);
        simulateMockFill();
      } catch (e) {
        showToast(`Error: ${e}`);
        onBufferingChange(true);
        simulateMockFill();
      }
    }
  }

  function simulateMockFill() {
    let v = 0;
    const id = setInterval(() => {
      v = Math.min(v + 0.003, 1);
      setMockPct(v);
      if (v >= 1) clearInterval(id);
    }, 200);
  }

  const handleSave = useCallback(
    async (continueRec: boolean) => {
      setSaving(true);
      try {
        const path = await saveClip(continueRec);
        if (path.startsWith("RECORDING_STARTED:")) {
          showToast("🔴 Recording started to disk");
        } else {
          showToast(`Saved: ${path.split(/[\\/]/).pop()}`);
          if (!continueRec) {
            onBufferingChange(false);
            onStatsChange({ used_mb: 0, limit_mb: settings.ram_limit_mb, duration_seconds: 0, is_recording_to_disk: false });
            setMockPct(0);
          }
        }
      } catch (e) {
        showToast(`Action failed: ${e}`);
      } finally {
        setSaving(false);
      }
    },
    [onBufferingChange, onStatsChange, settings.ram_limit_mb]
  );

  return (
    <div className="main-page fade-in">
      <div className="arc-section">
        <svg
          className="arc-svg"
          width={SIZE}
          height={SIZE}
          style={{ transform: `rotate(${START_ANGLE}deg)` }}
        >
          <circle
            cx={SIZE / 2}
            cy={SIZE / 2}
            r={R}
            fill="none"
            stroke="rgba(255,255,255,0.06)"
            strokeWidth={STROKE}
            strokeDasharray={`${ARC_LEN} ${CIRCUMFERENCE - ARC_LEN}`}
            strokeLinecap="round"
          />
          <circle
            cx={SIZE / 2}
            cy={SIZE / 2}
            r={R}
            fill="none"
            stroke={arcColor}
            strokeWidth={STROKE}
            strokeDasharray={`${Math.max(ARC_LEN - dashOffset, 0)} ${CIRCUMFERENCE}`}
            strokeLinecap="round"
            style={{
              transition: "stroke-dasharray 0.6s cubic-bezier(.4,0,.2,1), stroke 0.5s ease",
              filter: `drop-shadow(0 0 10px ${arcColor}99)`,
            }}
          />
        </svg>

        <div className="arc-center">
          {isRecording ? (
            <>
              <div className="arc-pct" style={{ color: "#ef4444", display: "inline-flex", alignItems: "center", gap: 6, fontSize: 24, fontWeight: 700 }}>
                <span className="rec-dot-pulse" style={{ width: 10, height: 10 }} />
                REC
              </div>
              <div className="arc-sub" style={{ color: "#f87171", fontWeight: 600 }}>
                {formatDuration(stats.duration_seconds)}
              </div>
              <div className="arc-sub arc-mb">
                {formatMb(stats.used_mb)} disk
              </div>
            </>
          ) : (
            <>
              <div className="arc-pct" style={{ color: arcColor }}>
                {(displayPct * 100).toFixed(0)}%
              </div>
              <div className="arc-sub">
                {formatDuration(stats.duration_seconds)}
              </div>
              <div className="arc-sub arc-mb">
                {formatMb(stats.used_mb)}
              </div>
            </>
          )}
        </div>

        {isBuffering && (
          <div className="arc-pulse-ring" style={{ borderColor: `${arcColor}44` }} />
        )}
      </div>

      <div className="main-toggle-row">
        <button
          className={`toggle-btn ${isBuffering ? "toggle-btn--stop" : "toggle-btn--start"}`}
          tabIndex={-1}
          onClick={(e) => {
            (e.currentTarget as HTMLButtonElement).blur();
            handleToggle();
          }}
          onKeyDown={(e) => {
            if (e.code === "Space" || e.code === "Enter") {
              e.preventDefault();
            }
          }}
        >
          {isBuffering ? (
            <>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
                <rect x="6" y="6" width="12" height="12" rx="2"/>
              </svg>
              {isRecording ? "Stop Recording & Buffer" : "Stop Buffering"}
            </>
          ) : (
            <>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
                <circle cx="12" cy="12" r="8"/>
              </svg>
              Start Buffering
            </>
          )}
        </button>
      </div>

      <div className="main-actions">
        {isRecording ? (
          <>
            <button
              className="pill-btn pill-btn-recording"
              tabIndex={-1}
              disabled={saving}
              onClick={(e) => {
                (e.currentTarget as HTMLButtonElement).blur();
                handleSave(true);
              }}
              title="Stop continuous disk recording, save video file, and resume buffer (F9)"
            >
              <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor">
                <rect x="6" y="6" width="12" height="12" rx="2" />
              </svg>
              {saving ? "Finalizing..." : "Stop Recording"}
            </button>
            <button
              className="pill-btn pill-btn-danger"
              tabIndex={-1}
              disabled={saving}
              onClick={(e) => {
                (e.currentTarget as HTMLButtonElement).blur();
                handleSave(false);
              }}
              title="Stop recording and stop buffering completely (F10)"
            >
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="10" />
                <line x1="15" y1="9" x2="9" y2="15" />
                <line x1="9" y1="9" x2="15" y2="15" />
              </svg>
              {saving ? "Finalizing..." : "Stop & Exit"}
            </button>
          </>
        ) : (
          <>
            <button
              className="pill-btn pill-btn-success"
              tabIndex={-1}
              disabled={!isBuffering || saving}
              onClick={(e) => {
                (e.currentTarget as HTMLButtonElement).blur();
                handleSave(true);
              }}
              title="Start recording to disk with buffer pre-roll (F9)"
            >
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"/>
                <polyline points="17 21 17 13 7 13 7 21"/>
              </svg>
              {saving ? "Starting..." : "Save + Continue"}
            </button>
            <button
              className="pill-btn pill-btn-danger"
              tabIndex={-1}
              disabled={!isBuffering || saving}
              onClick={(e) => {
                (e.currentTarget as HTMLButtonElement).blur();
                handleSave(false);
              }}
              title="Save buffered replay to disk and stop buffering (F10)"
            >
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="10"/>
                <line x1="15" y1="9" x2="9" y2="15"/>
                <line x1="9" y1="9" x2="15" y2="15"/>
              </svg>
              {saving ? "Saving..." : "Save + Stop"}
            </button>
          </>
        )}
      </div>

      <div className="main-info-row">
        <BufferBar stats={stats.used_mb > 0 ? stats : { ...stats, used_mb: displayPct * stats.limit_mb }} />
      </div>

      <div className="main-status-grid">
        <div className="status-card">
          <span className="status-label">Codec</span>
          <span className="status-value">{settings.codec}</span>
        </div>
        <div className="status-card">
          <span className="status-label">FPS</span>
          <span className="status-value">{settings.fps}</span>
        </div>
        <div className="status-card">
          <span className="status-label">Resolution</span>
          <span className="status-value">{settings.resolution}</span>
        </div>
        <div className="status-card">
          <span className="status-label">Toggle Buffer</span>
          <span className="status-value hotkey-badge">{settings.hotkey_toggle_buffer || "Alt+Shift+F8"}</span>
        </div>
        <div className="status-card">
          <span className="status-label">{isRecording ? "Stop Rec" : "Save + Continue"}</span>
          <span className="status-value hotkey-badge">{settings.hotkey_save_continue}</span>
        </div>
        <div className="status-card">
          <span className="status-label">Save + Stop</span>
          <span className="status-value hotkey-badge">{settings.hotkey_save_stop}</span>
        </div>
      </div>

      {toast && <div className="toast">{toast}</div>}

      <style>{`
        .main-page {
          display: flex;
          flex-direction: column;
          align-items: center;
          gap: 14px;
          padding: 16px 20px 20px;
          width: 100%;
          box-sizing: border-box;
          overflow: hidden;
        }
        .arc-section {
          position: relative;
          width: ${SIZE}px;
          height: ${SIZE}px;
          display: flex;
          align-items: center;
          justify-content: center;
        }
        .arc-svg {
          position: absolute;
          inset: 0;
        }
        .arc-center {
          display: flex;
          flex-direction: column;
          align-items: center;
          gap: 2px;
          z-index: 1;
        }
        .arc-pct {
          font-size: 36px;
          font-weight: 700;
          letter-spacing: -0.04em;
          line-height: 1;
          font-variant-numeric: tabular-nums;
          transition: color 0.5s ease;
        }
        .arc-sub {
          font-size: 13px;
          color: var(--text-muted);
          font-variant-numeric: tabular-nums;
          font-weight: 500;
        }
        .arc-mb {
          font-size: 11px;
          color: var(--text-dim);
        }
        .arc-pulse-ring {
          position: absolute;
          inset: -12px;
          border-radius: 50%;
          border: 2px solid;
          animation: pulse-ring 2s ease infinite;
          pointer-events: none;
        }
        .main-toggle-row {
          display: flex;
          justify-content: center;
        }
        .toggle-btn {
          display: inline-flex;
          align-items: center;
          gap: 8px;
          padding: 13px 32px;
          border-radius: var(--radius-pill);
          font-size: 15px;
          font-weight: 700;
          letter-spacing: -0.01em;
          transition: all 0.2s ease;
        }
        .toggle-btn--start {
          background: var(--accent);
          color: #fff;
          box-shadow: 0 4px 24px var(--accent-glow);
        }
        .toggle-btn--start:hover {
          background: var(--accent-hover);
          box-shadow: 0 6px 32px var(--accent-glow);
          transform: translateY(-1px);
        }
        .toggle-btn--stop {
          background: rgba(239,68,68,0.15);
          color: var(--danger);
          border: 1px solid rgba(239,68,68,0.3);
          box-shadow: 0 4px 20px rgba(239,68,68,0.15);
        }
        .toggle-btn--stop:hover {
          background: rgba(239,68,68,0.25);
          transform: translateY(-1px);
        }
        .main-actions {
          display: flex;
          gap: 10px;
          width: 100%;
          justify-content: center;
        }
        .main-actions .pill-btn {
          flex: 1;
          max-width: 200px;
          justify-content: center;
        }
        .main-info-row {
          width: 100%;
        }
        .main-status-grid {
          display: grid;
          grid-template-columns: repeat(3, 1fr);
          gap: 8px;
          width: 100%;
        }
        .status-card {
          display: flex;
          flex-direction: column;
          gap: 3px;
          padding: 10px 12px;
          background: var(--surface);
          border: 1px solid var(--border);
          border-radius: var(--radius-sm);
        }
        .status-label {
          font-size: 10px;
          font-weight: 600;
          text-transform: uppercase;
          letter-spacing: 0.06em;
          color: var(--text-dim);
        }
        .status-value {
          font-size: 12px;
          font-weight: 600;
          color: var(--text);
          white-space: nowrap;
          overflow: hidden;
          text-overflow: ellipsis;
        }
        .hotkey-badge {
          color: var(--accent);
        }
      `}</style>
    </div>
  );
}
