import React, { useState, useRef, useCallback } from "react";

interface Props {
  value: string;
  onChange: (key: string) => void;
  placeholder?: string;
}

const MODIFIER_KEYS = new Set(["Control", "Alt", "Shift", "Meta"]);

export default function HotkeyInput({ value, onChange, placeholder = "Click to record..." }: Props) {
  const [isListening, setIsListening] = useState(false);
  const inputRef = useRef<HTMLDivElement>(null);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();

      if (e.key === "Escape") {
        setIsListening(false);
        inputRef.current?.blur();
        return;
      }

      if (MODIFIER_KEYS.has(e.key)) return;

      const parts: string[] = [];
      if (e.ctrlKey) parts.push("Ctrl");
      if (e.altKey) parts.push("Alt");
      if (e.shiftKey) parts.push("Shift");

      const key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
      parts.push(key);

      onChange(parts.join("+"));
      setIsListening(false);
      inputRef.current?.blur();
    },
    [onChange]
  );

  return (
    <div
      ref={inputRef}
      className={`hotkey-input ${isListening ? "hotkey-input--listening" : ""}`}
      tabIndex={0}
      onFocus={() => setIsListening(true)}
      onBlur={() => setIsListening(false)}
      onKeyDown={handleKeyDown}
      role="textbox"
      aria-label="Hotkey input"
    >
      {isListening ? (
        <span className="hotkey-hint">Press a key combination…</span>
      ) : value ? (
        <div className="hotkey-keys">
          {value.split("+").map((k) => (
            <kbd key={k}>{k}</kbd>
          ))}
        </div>
      ) : (
        <span className="hotkey-hint">{placeholder}</span>
      )}

      <style>{`
        .hotkey-input {
          display: flex;
          align-items: center;
          gap: 4px;
          min-height: 36px;
          padding: 6px 10px;
          background: var(--surface-2);
          border: 1px solid var(--border);
          border-radius: var(--radius-sm);
          cursor: pointer;
          transition: border-color var(--transition), box-shadow var(--transition);
        }
        .hotkey-input:focus {
          outline: none;
        }
        .hotkey-input--listening {
          border-color: var(--accent);
          box-shadow: 0 0 0 3px var(--accent-subtle);
        }
        .hotkey-hint {
          font-size: 12px;
          color: var(--text-dim);
          font-style: italic;
        }
        .hotkey-keys {
          display: flex;
          gap: 4px;
          flex-wrap: wrap;
        }
        kbd {
          font-family: inherit;
          font-size: 12px;
          font-weight: 600;
          color: var(--text);
          background: var(--surface-3);
          border: 1px solid rgba(255,255,255,0.1);
          border-radius: 5px;
          padding: 2px 7px;
          line-height: 1.4;
        }
      `}</style>
    </div>
  );
}
