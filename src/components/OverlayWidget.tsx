import React, { useMemo } from "react";
import type { BufferStats } from "../lib/types";
import { saveClip } from "../lib/tauri";

interface Props {
  stats: BufferStats;
  isBuffering: boolean;
}

const SIZE = 80;
const STROKE = 7;
const R = (SIZE - STROKE) / 2;
const CIRCUMFERENCE = 2 * Math.PI * R;
const GAP_DEG = 60;
const ARC_DEG = 360 - GAP_DEG;

export default function OverlayWidget({ stats, isBuffering }: Props) {
  const pct = useMemo(
    () => (stats.limit_mb > 0 ? Math.min(stats.used_mb / stats.limit_mb, 1) : 0),
    [stats]
  );

  const dashOffset = useMemo(
    () => CIRCUMFERENCE * (ARC_DEG / 360) * (1 - pct),
    [pct]
  );

  const arcLen = CIRCUMFERENCE * (ARC_DEG / 360);
  const startAngle = 90 + GAP_DEG / 2;

  const color = pct > 0.9 ? "#ef4444" : pct > 0.7 ? "#f59e0b" : "#7c3aed";

  async function handleSave() {
    try {
      await saveClip(true);
    } catch (_) {}
  }

  return (
    <div className="overlay-wrap">
      <div className="overlay-arc-wrap">
        <svg width={SIZE} height={SIZE} style={{ transform: `rotate(${startAngle}deg)` }}>
          <circle
            cx={SIZE / 2}
            cy={SIZE / 2}
            r={R}
            fill="none"
            stroke="rgba(255,255,255,0.08)"
            strokeWidth={STROKE}
            strokeDasharray={`${arcLen} ${CIRCUMFERENCE - arcLen}`}
            strokeLinecap="round"
          />
          <circle
            cx={SIZE / 2}
            cy={SIZE / 2}
            r={R}
            fill="none"
            stroke={color}
            strokeWidth={STROKE}
            strokeDasharray={`${arcLen - dashOffset} ${CIRCUMFERENCE - (arcLen - dashOffset)}`}
            strokeLinecap="round"
            style={{ transition: "stroke-dasharray 0.5s ease, stroke 0.5s ease", filter: `drop-shadow(0 0 6px ${color})` }}
          />
        </svg>
        <div className="overlay-center">
          <div className={`overlay-dot ${isBuffering ? "overlay-dot--active" : ""}`} />
        </div>
      </div>
      <button className="overlay-save-btn" onClick={handleSave} disabled={!isBuffering} title="Save clip">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"/>
          <polyline points="17 21 17 13 7 13 7 21"/>
          <polyline points="7 3 7 8 15 8"/>
        </svg>
      </button>

      <style>{`
        .overlay-wrap {
          display: flex;
          flex-direction: column;
          align-items: center;
          gap: 8px;
          padding: 10px;
          background: rgba(13, 13, 15, 0.92);
          backdrop-filter: blur(16px);
          -webkit-backdrop-filter: blur(16px);
          border: 1px solid rgba(255,255,255,0.08);
          border-radius: 18px;
          width: fit-content;
        }
        .overlay-arc-wrap {
          position: relative;
          width: ${SIZE}px;
          height: ${SIZE}px;
        }
        .overlay-center {
          position: absolute;
          inset: 0;
          display: flex;
          align-items: center;
          justify-content: center;
        }
        .overlay-dot {
          width: 10px;
          height: 10px;
          border-radius: 50%;
          background: var(--surface-3);
          transition: background 0.3s ease, box-shadow 0.3s ease;
        }
        .overlay-dot--active {
          background: var(--danger);
          box-shadow: 0 0 10px rgba(239,68,68,0.6);
          animation: pulse-ring 1.5s ease infinite;
        }
        .overlay-save-btn {
          display: flex;
          align-items: center;
          justify-content: center;
          width: 32px;
          height: 32px;
          border-radius: 50%;
          background: var(--accent-subtle);
          color: var(--accent);
          border: 1px solid rgba(124,58,237,0.3);
          transition: all var(--transition);
        }
        .overlay-save-btn:hover:not(:disabled) {
          background: var(--accent);
          color: #fff;
          box-shadow: 0 4px 16px var(--accent-glow);
        }
        .overlay-save-btn:disabled {
          opacity: 0.3;
          cursor: not-allowed;
        }
      `}</style>
    </div>
  );
}
