import React from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Page } from "../lib/types";

interface Props {
  activePage: Page;
  onNavigate: (page: Page) => void;
  isBuffering: boolean;
}

export default function TitleBar({ activePage, onNavigate, isBuffering }: Props) {
  const handleClose = (e: React.MouseEvent) => {
    e.stopPropagation();
    invoke("app_close").catch(() => {
      try {
        getCurrentWindow().close();
      } catch {
        window.close();
      }
    });
  };

  const handleMinimize = (e: React.MouseEvent) => {
    e.stopPropagation();
    invoke("app_minimize").catch(() => {
      try {
        getCurrentWindow().minimize();
      } catch {}
    });
  };

  const handleMouseDown = (e: React.MouseEvent) => {
    if (e.button === 0) {
      invoke("app_start_drag").catch(() => {});
    }
  };

  return (
    <div className="titlebar" onMouseDown={handleMouseDown}>
      <div className="titlebar-left">
        <div className={`titlebar-indicator ${isBuffering ? "active" : ""}`} />
        <span className="titlebar-logo">
          Specter
        </span>
      </div>
      <nav className="titlebar-nav" onMouseDown={(e) => e.stopPropagation()}>
        <button
          className={`nav-btn ${activePage === "main" ? "nav-btn--active" : ""}`}
          onClick={() => onNavigate("main")}
          onMouseDown={(e) => e.stopPropagation()}
        >
          Buffer
        </button>
        <button
          className={`nav-btn ${activePage === "settings" ? "nav-btn--active" : ""}`}
          onClick={() => onNavigate("settings")}
          onMouseDown={(e) => e.stopPropagation()}
        >
          Settings
        </button>
      </nav>
      <div className="titlebar-controls" onMouseDown={(e) => e.stopPropagation()}>
        <button
          className="wm-btn wm-btn-minimize"
          onClick={handleMinimize}
          onMouseDown={(e) => e.stopPropagation()}
          title="Minimize"
        >
          <svg width="10" height="2" viewBox="0 0 10 2" fill="currentColor">
            <rect width="10" height="1.5" rx="0.75"/>
          </svg>
        </button>
        <button
          className="wm-btn wm-btn-close"
          onClick={handleClose}
          onMouseDown={(e) => e.stopPropagation()}
          title="Close"
        >
          <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
            <path d="M1 1l8 8M9 1l-8 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/>
          </svg>
        </button>
      </div>

      <style>{`
        .titlebar {
          display: flex;
          align-items: center;
          gap: 12px;
          height: 44px;
          padding: 0 12px;
          background: var(--surface);
          border-bottom: 1px solid var(--border);
          flex-shrink: 0;
          user-select: none;
          -webkit-user-select: none;
        }
        .titlebar-left {
          display: flex;
          align-items: center;
          gap: 8px;
          flex: 1;
        }
        .titlebar-indicator {
          width: 7px;
          height: 7px;
          border-radius: 50%;
          background: var(--surface-3);
          transition: background 0.3s ease, box-shadow 0.3s ease;
        }
        .titlebar-indicator.active {
          background: var(--danger);
          box-shadow: 0 0 8px rgba(239, 68, 68, 0.6);
          animation: pulse-ring 2s ease infinite;
        }
        .titlebar-logo {
          font-size: 14px;
          font-weight: 700;
          letter-spacing: -0.02em;
          color: var(--text);
          cursor: default;
        }
        .titlebar-nav {
          display: flex;
          gap: 2px;
          background: var(--bg);
          border-radius: var(--radius-pill);
          padding: 3px;
        }
        .nav-btn {
          padding: 4px 14px;
          border-radius: var(--radius-pill);
          font-size: 12px;
          font-weight: 500;
          color: var(--text-dim);
          background: transparent;
          transition: all var(--transition);
        }
        .nav-btn:hover {
          color: var(--text-muted);
        }
        .nav-btn--active {
          background: var(--surface-2);
          color: var(--text);
        }
        .titlebar-controls {
          display: flex;
          gap: 6px;
          margin-left: auto;
        }
        .wm-btn {
          display: flex;
          align-items: center;
          justify-content: center;
          width: 26px;
          height: 26px;
          border-radius: 50%;
          background: var(--surface-3);
          color: var(--text-dim);
          transition: all var(--transition);
        }
        .wm-btn:hover {
          color: var(--text);
        }
        .wm-btn-close:hover {
          background: rgba(239, 68, 68, 0.3);
          color: var(--danger);
        }
        .wm-btn-minimize:hover {
          background: rgba(245, 158, 11, 0.3);
          color: var(--warning);
        }
      `}</style>
    </div>
  );
}
