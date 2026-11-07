import React, { useMemo } from "react";
import type { BufferStats } from "../lib/types";

interface Props {
  stats: BufferStats;
}

function formatDuration(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function formatMb(mb: number): string {
  if (mb >= 1000) return `${(mb / 1024).toFixed(1)} GB`;
  return `${Math.round(mb)} MB`;
}

export default function BufferBar({ stats }: Props) {
  const pct = useMemo(
    () => (stats.limit_mb > 0 ? Math.min((stats.used_mb / stats.limit_mb) * 100, 100) : 0),
    [stats]
  );

  const isRecording = Boolean(stats.is_recording_to_disk);

  const barColor = useMemo(() => {
    if (isRecording) return "#ef4444";
    if (pct > 90) return "var(--danger)";
    if (pct > 70) return "var(--warning)";
    return "var(--accent)";
  }, [pct, isRecording]);

  return (
    <div className="buffer-bar-wrap">
      <div className="buffer-bar-header">
        <span
          className="buffer-bar-label"
          style={isRecording ? { color: "#ef4444", display: "inline-flex", alignItems: "center", gap: 6 } : undefined}
        >
          {isRecording && <span className="rec-dot-pulse" />}
          {isRecording ? "RECORDING DIRECTLY TO DISK" : "RAM Buffer"}
        </span>
        <span className="buffer-bar-meta">
          {isRecording
            ? `${formatDuration(stats.duration_seconds)} · ${formatMb(stats.used_mb)} written`
            : `${formatDuration(stats.duration_seconds)} · ${formatMb(stats.used_mb)} / ${formatMb(stats.limit_mb)}`}
        </span>
      </div>
      <div className="buffer-track">
        <div
          className="buffer-fill"
          style={{ width: isRecording ? "100%" : `${pct}%`, background: barColor }}
        />
        <div
          className="buffer-glow"
          style={{ width: isRecording ? "100%" : `${pct}%`, background: barColor }}
        />
      </div>
      <div className="buffer-pct" style={{ color: barColor }}>
        {isRecording ? "REC" : `${pct.toFixed(1)}%`}
      </div>

      <style>{`
        .buffer-bar-wrap {
          display: flex;
          flex-direction: column;
          gap: 6px;
          padding: 14px 16px;
          background: var(--surface);
          border: 1px solid var(--border);
          border-radius: var(--radius);
        }
        .buffer-bar-header {
          display: flex;
          justify-content: space-between;
          align-items: center;
        }
        .buffer-bar-label {
          font-size: 11px;
          font-weight: 600;
          text-transform: uppercase;
          letter-spacing: 0.08em;
          color: var(--text-dim);
        }
        .buffer-bar-meta {
          font-size: 12px;
          color: var(--text-muted);
          font-variant-numeric: tabular-nums;
        }
        .buffer-track {
          position: relative;
          height: 6px;
          background: var(--surface-3);
          border-radius: 3px;
          overflow: hidden;
        }
        .buffer-fill {
          height: 100%;
          border-radius: 3px;
          transition: width 0.5s ease, background 0.5s ease;
        }
        .buffer-glow {
          position: absolute;
          top: 0;
          left: 0;
          height: 100%;
          border-radius: 3px;
          opacity: 0.3;
          filter: blur(4px);
          transition: width 0.5s ease, background 0.5s ease;
        }
        .buffer-pct {
          font-size: 12px;
          font-weight: 600;
          font-variant-numeric: tabular-nums;
          align-self: flex-end;
          transition: color 0.5s ease;
        }
      `}</style>
    </div>
  );
}
