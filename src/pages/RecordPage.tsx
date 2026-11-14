import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  Monitor,
  RecorderStats,
  RecordingAudio,
  RecordingConfig,
  RecordingMode,
  RecordingQuality,
  Settings,
  WindowInfo,
} from "../lib/types";
import {
  capturePreview,
  getAudioDevices,
  getMonitors,
  getWindows,
  onRecorderLevel,
  saveSettings,
  startAudioMeter,
  startRecording,
  stopAudioMeter,
  stopRecording,
  switchRecordingSource,
} from "../lib/tauri";

interface Props {
  settings: Settings;
  settingsLoaded: boolean;
  isRecording: boolean;
  stats: RecorderStats;
  warning: string | null;
  onRecordingChange: (v: boolean) => void;
  onStatsChange: (s: RecorderStats) => void;
  onSettingsChange: (s: Settings) => void;
}

const RESOLUTIONS = [
  { value: "source", label: "Source" },
  { value: "720p", label: "720p" },
  { value: "1080p", label: "1080p" },
  { value: "1440p", label: "1440p" },
];

const FRAME_RATES = [30, 48, 60, 120, 144];

const QUALITIES: { value: RecordingQuality; label: string }[] = [
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "ultra", label: "Ultra" },
];

const AUDIO_OPTIONS: { value: RecordingAudio; label: string }[] = [
  { value: "none", label: "None" },
  { value: "mic", label: "Mic" },
  { value: "system", label: "System" },
  { value: "both", label: "Both" },
];

const CODECS = [
  { value: "h264_nvenc", label: "H.264 NVENC (GPU)" },
  { value: "hevc_nvenc", label: "H.265 NVENC (GPU)" },
  { value: "h264_amf", label: "H.264 AMF (AMD)" },
  { value: "h264_qsv", label: "H.264 Quick Sync (Intel)" },
  { value: "libx264", label: "H.264 CPU Fallback" },
];

const WAVE_BARS = 110;
const LOOPBACK_RE = /stereo mix|loopback|what u hear|cable output|virtual-audio/i;

function formatDuration(secs: number): string {
  const total = Math.max(0, Math.floor(secs));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = m.toString().padStart(2, "0");
  const ss = s.toString().padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

function formatSize(mb: number): string {
  if (mb >= 1024) return `${(mb / 1024).toFixed(2)} GB`;
  if (mb < 10) return `${mb.toFixed(1)} MB`;
  return `${Math.round(mb)} MB`;
}

// Mirrors `quality_bitrate_mbps` in `src-tauri/src/recorder.rs`, used only for
// the informational estimate shown in the UI.
function estimateBitrateMbps(resolution: string, fps: number, quality: string): number {
  const base =
    resolution === "720p" ? 4 : resolution === "1440p" ? 14 : resolution === "1080p" ? 7 : 8;
  const fpsFactor = Math.min(1.6, Math.max(0.6, Math.sqrt(Math.max(1, fps) / 60)));
  const qualityFactor =
    quality === "low" ? 0.6 : quality === "medium" ? 1 : quality === "high" ? 1.6 : 2.4;
  return Math.round(base * fpsFactor * qualityFactor * 10) / 10;
}

// Bounds an async call so a stuck backend can never leave the preview UI showing
// "Loading preview…" forever.
function withTimeout<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const id = window.setTimeout(() => reject(new Error(message)), ms);
    promise.then(
      (value) => {
        window.clearTimeout(id);
        resolve(value);
      },
      (error) => {
        window.clearTimeout(id);
        reject(error);
      }
    );
  });
}

function needsMic(audio: RecordingAudio): boolean {
  return audio === "mic" || audio === "both";
}

function needsSystem(audio: RecordingAudio): boolean {
  return audio === "system" || audio === "both";
}

interface Draft {
  mode: RecordingMode;
  resolution: string;
  fps: number;
  quality: RecordingQuality;
  audio: RecordingAudio;
  codec: string;
  micDevice: string;
  sourceKind: "screen" | "window";
  sourceValue: string;
}

function parseSource(raw: string): { sourceKind: "screen" | "window"; sourceValue: string } {
  if (raw.startsWith("window:")) {
    return { sourceKind: "window", sourceValue: raw.slice("window:".length) };
  }
  return { sourceKind: "screen", sourceValue: raw.replace(/^screen:/, "") || "0" };
}

function draftFromSettings(settings: Settings): Draft {
  const mode: RecordingMode = settings.recording_mode === "audio" ? "audio" : "video";
  const audio = AUDIO_OPTIONS.some((o) => o.value === settings.recording_audio)
    ? (settings.recording_audio as RecordingAudio)
    : "none";
  return {
    mode,
    resolution: settings.recording_resolution || "1080p",
    fps: settings.recording_fps || 60,
    quality: (QUALITIES.some((q) => q.value === settings.recording_quality)
      ? settings.recording_quality
      : "high") as RecordingQuality,
    audio: mode === "audio" && audio === "none" ? "mic" : audio,
    codec: settings.recording_codec || "h264_nvenc",
    micDevice: settings.mic_device || "",
    ...parseSource(settings.capture_source || "screen:0"),
  };
}

/// Projects the Record tab draft onto the persisted settings fields. Keeping this
/// in one place lets the tab save every change immediately, so choices survive
/// navigation and restarts instead of living only in component state.
function mergeDraft(settings: Settings, draft: Draft): Settings {
  return {
    ...settings,
    recording_mode: draft.mode,
    recording_resolution: draft.resolution,
    recording_fps: draft.fps,
    recording_quality: draft.quality,
    recording_audio: draft.audio,
    recording_codec: draft.codec,
    mic_device: draft.micDevice,
    capture_source: `${draft.sourceKind}:${draft.sourceValue}`,
  };
}

function recordingSettingsEqual(a: Settings, b: Settings): boolean {
  return (
    a.recording_mode === b.recording_mode &&
    a.recording_resolution === b.recording_resolution &&
    a.recording_fps === b.recording_fps &&
    a.recording_quality === b.recording_quality &&
    a.recording_audio === b.recording_audio &&
    a.recording_codec === b.recording_codec &&
    a.mic_device === b.mic_device &&
    a.capture_source === b.capture_source
  );
}

function Segment<T extends string | number>({
  value,
  options,
  onChange,
  disabled,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  disabled?: boolean;
}) {
  return (
    <div className={`rec-seg ${disabled ? "rec-seg--disabled" : ""}`}>
      {options.map((option) => (
        <button
          key={String(option.value)}
          className={`rec-seg-btn ${option.value === value ? "rec-seg-btn--active" : ""}`}
          onClick={() => onChange(option.value)}
          disabled={disabled}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export default function RecordPage({
  settings,
  settingsLoaded,
  isRecording,
  stats,
  warning,
  onRecordingChange,
  onStatsChange,
  onSettingsChange,
}: Props) {
  const [draft, setDraft] = useState<Draft>(() => draftFromSettings(settings));
  const [devices, setDevices] = useState<string[]>([]);
  const [monitors, setMonitors] = useState<Monitor[]>([]);
  const [windows, setWindows] = useState<WindowInfo[]>([]);
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [previewOn, setPreviewOn] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);

  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const levelRef = useRef(0);
  const historyRef = useRef<number[]>([]);
  const recordingRef = useRef(isRecording);
  recordingRef.current = isRecording;

  useEffect(() => {
    if (settingsLoaded && !isRecording) setDraft(draftFromSettings(settings));
    // Only re-sync when saved settings change, not when a recording ends,
    // so the user's tweaks survive a start/stop cycle.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings, settingsLoaded]);

  // Settings are persisted from explicit user actions only. An effect that saved
  // whenever the draft differed used to run with the default settings before the
  // real ones finished loading and could overwrite the config file.
  const persistDraft = useCallback(
    (next: Draft) => {
      if (!settingsLoaded) return;
      const merged = mergeDraft(settings, next);
      if (recordingSettingsEqual(merged, settings)) return;
      onSettingsChange(merged);
      void saveSettings(merged).catch((error) => {
        setToast(`Settings were not saved: ${error instanceof Error ? error.message : String(error)}`);
      });
    },
    [settings, settingsLoaded, onSettingsChange]
  );

  useEffect(() => {
    getAudioDevices()
      .then((list) => {
        setDevices(list);
        setDraft((prev) => {
          if (prev.micDevice && list.includes(prev.micDevice)) return prev;
          return { ...prev, micDevice: list[0] ?? "" };
        });
      })
      .catch(() => {});
  }, []);

  useEffect(() => onRecorderLevel((level) => (levelRef.current = level)), []);

  // Live level meter for the selected audio source, active only while the tab is
  // open and nothing is being recorded. Without it a muted device looks identical
  // to a broken waveform, so the user cannot tell why the bar never moves.
  useEffect(() => {
    if (!settingsLoaded || isRecording) return;
    if (!needsMic(draft.audio) && !needsSystem(draft.audio)) return;
    let cancelled = false;
    startAudioMeter(draft.audio, draft.micDevice).catch(() => {
      if (!cancelled) levelRef.current = 0;
    });
    return () => {
      cancelled = true;
      levelRef.current = 0;
      void stopAudioMeter();
    };
  }, [settingsLoaded, isRecording, draft.audio, draft.micDevice]);

  // Low-rate preview of the selected source. It is paused while recording so it
  // never competes with the capture pipeline.
  useEffect(() => {
    if (!previewOn || isRecording || draft.mode === "audio") {
      setPreviewUrl((prev) => {
        if (prev) URL.revokeObjectURL(prev);
        return null;
      });
      setPreviewError(null);
      return;
    }
    let cancelled = false;
    let inflight = false;
    const source = `${draft.sourceKind}:${draft.sourceValue}`;
    let timer = 0;

    const tick = async () => {
      if (cancelled || inflight) return;
      inflight = true;
      try {
        const blob = await withTimeout(
          capturePreview(source, settings.capture_cursor, settings.capture_backend, 480),
          8000,
          "capture timed out"
        );
        if (!blob) throw new Error("capture returned no frame");
        if (!cancelled) {
          setPreviewError(null);
          const url = URL.createObjectURL(blob);
          setPreviewUrl((prev) => {
            if (prev) URL.revokeObjectURL(prev);
            return url;
          });
        }
      } catch (error) {
        // Preview is best-effort; surface the reason instead of hanging on
        // "Loading preview…" forever, and keep retrying on the next tick.
        if (!cancelled) {
          setPreviewError(error instanceof Error ? error.message : String(error));
        }
      } finally {
        inflight = false;
      }
    };

    void tick();
    timer = window.setInterval(tick, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [
    previewOn,
    isRecording,
    draft.mode,
    draft.sourceKind,
    draft.sourceValue,
    settings.capture_cursor,
    settings.capture_backend,
  ]);

  function togglePreview(next: boolean) {
    setPreviewOn(next);
  }

  const refreshWindows = useCallback(async () => {
    try {
      setWindows(await getWindows());
    } catch {
      // Window enumeration is best-effort; keep the previous list on failure.
    }
  }, []);

  useEffect(() => {
    getMonitors()
      .then(setMonitors)
      .catch(() => {});
    void refreshWindows();
  }, [refreshWindows]);

  // Keep the selection valid when the available displays/windows change. This
  // only runs outside a recording so it never overrides a deliberate switch.
  useEffect(() => {
    if (isRecording || draft.sourceKind !== "screen" || monitors.length === 0) return;
    if (monitors.some((m) => m.id === `screen:${draft.sourceValue}`)) return;
    setDraft((prev) => ({ ...prev, sourceValue: monitors[0].id.slice("screen:".length) }));
  }, [monitors, draft.sourceKind, draft.sourceValue, isRecording]);

  useEffect(() => {
    if (isRecording || draft.sourceKind !== "window" || windows.length === 0) return;
    if (windows.some((w) => String(w.pid) === draft.sourceValue)) return;
    setDraft((prev) => ({ ...prev, sourceValue: String(windows[0].pid) }));
  }, [windows, draft.sourceKind, draft.sourceValue, isRecording]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const history = historyRef.current;
    let raf = 0;
    let lastSample = 0;
    let peak = 0.2;

    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      const rect = canvas.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return;
      canvas.width = Math.round(rect.width * dpr);
      canvas.height = Math.round(rect.height * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(canvas);

    const render = (time: number) => {
      raf = requestAnimationFrame(render);

      if (time - lastSample >= 36) {
        lastSample = time;
        const raw = Math.min(1, Math.max(0, levelRef.current));
        // Decay the stored level so the wave falls back to zero as soon as the
        // meter stops emitting (recording ended or the device went silent).
        levelRef.current = raw * 0.88;
        // Auto-gain: normalize against a slowly decaying peak so quiet
        // microphones still produce a visible swing.
        peak = Math.max(raw, peak * 0.996, 0.2);
        const scaled = Math.min(1, raw / peak);
        const prev = history.length > 0 ? history[history.length - 1] : 0;
        // Attack fast, release slow so the wave breathes instead of flickering.
        const next = scaled > prev ? scaled : prev * 0.82 + scaled * 0.18;
        history.push(next);
        if (history.length > WAVE_BARS) history.shift();
      }

      const rect = canvas.getBoundingClientRect();
      const width = rect.width;
      const height = rect.height;
      if (width === 0 || height === 0) return;
      const mid = height / 2;
      const active = recordingRef.current;

      ctx.clearRect(0, 0, width, height);

      ctx.beginPath();
      ctx.moveTo(0, mid);
      ctx.lineTo(width, mid);
      ctx.strokeStyle = active ? "rgba(239,68,68,0.16)" : "rgba(255,255,255,0.05)";
      ctx.lineWidth = 1;
      ctx.stroke();

      if (history.length < 2) return;

      const step = width / (WAVE_BARS - 1);
      const points = history.map((sample, i) => {
        const x = width - (history.length - 1 - i) * step;
        const amplitude = Math.pow(sample, 0.7) * (mid - 4);
        return { x, y: mid - amplitude };
      });

      const trace = (offset: number) => {
        ctx.beginPath();
        ctx.moveTo(points[0].x, mid + (points[0].y - mid) * offset);
        for (let i = 1; i < points.length - 1; i += 1) {
          const xc = (points[i].x + points[i + 1].x) / 2;
          const yc = mid + ((points[i].y + points[i + 1].y) / 2 - mid) * offset;
          ctx.quadraticCurveTo(
            points[i].x,
            mid + (points[i].y - mid) * offset,
            xc,
            yc
          );
        }
        const tail = points[points.length - 1];
        ctx.lineTo(tail.x, mid + (tail.y - mid) * offset);
      };

      const gradient = ctx.createLinearGradient(0, 0, width, 0);
      if (active) {
        gradient.addColorStop(0, "rgba(239,68,68,0.04)");
        gradient.addColorStop(0.55, "rgba(239,68,68,0.35)");
        gradient.addColorStop(1, "rgba(248,113,113,0.55)");
      } else {
        gradient.addColorStop(0, "rgba(124,58,237,0.05)");
        gradient.addColorStop(1, "rgba(124,58,237,0.3)");
      }

      trace(1);
      for (let i = points.length - 1; i >= 0; i -= 1) {
        ctx.lineTo(points[i].x, mid - (points[i].y - mid));
      }
      ctx.closePath();
      ctx.fillStyle = gradient;
      ctx.fill();

      ctx.save();
      ctx.shadowBlur = active ? 14 : 8;
      ctx.shadowColor = active ? "rgba(239,68,68,0.55)" : "rgba(124,58,237,0.35)";
      ctx.lineWidth = 2;
      ctx.strokeStyle = active ? "#f87171" : "#8b5cf6";
      trace(1);
      ctx.stroke();
      trace(-1);
      ctx.stroke();
      ctx.restore();
    };

    raf = requestAnimationFrame(render);
    return () => {
      cancelAnimationFrame(raf);
      observer.disconnect();
    };
  }, []);

  const patch = useCallback(
    <K extends keyof Draft>(key: K, value: Draft[K]) => {
      const next = { ...draft, [key]: value };
      setDraft(next);
      persistDraft(next);
    },
    [draft, persistDraft]
  );

  const buildConfig = useCallback(
    (overrides?: Partial<Draft>): RecordingConfig => {
      const d = { ...draft, ...overrides };
      return {
        mode: d.mode,
        resolution: d.resolution,
        fps: d.fps,
        quality: d.quality,
        audio: d.audio,
        mic_device: needsMic(d.audio) ? d.micDevice : "",
        codec: d.codec,
        capture_source: `${d.sourceKind}:${d.sourceValue}`,
        capture_cursor: settings.capture_cursor,
        capture_backend: settings.capture_backend,
        video_preset: settings.video_preset,
        output_folder: settings.output_folder,
      };
    },
    [
      draft,
      settings.capture_cursor,
      settings.capture_backend,
      settings.video_preset,
      settings.output_folder,
    ]
  );

  async function applyLive(overrides: Partial<Draft>, label: string) {
    if (!isRecording) return;
    try {
      await switchRecordingSource(buildConfig(overrides));
      showToast(`Switched: ${label}`);
    } catch (e) {
      showToast(`Switch failed: ${e}`);
    }
  }

  function patchLive<K extends keyof Draft>(key: K, value: Draft[K], label: string) {
    const next = { ...draft, [key]: value };
    setDraft(next);
    persistDraft(next);
    void applyLive({ [key]: value } as Partial<Draft>, label);
  }

  function chooseMode(mode: RecordingMode) {
    if (mode === draft.mode) return;
    const next: Draft = {
      ...draft,
      mode,
      audio: mode === "audio" && draft.audio === "none" ? "mic" : draft.audio,
    };
    setDraft(next);
    persistDraft(next);
    void applyLive({ mode, audio: next.audio }, mode === "audio" ? "audio only" : "video");
  }

  function chooseSourceKind(kind: "screen" | "window") {
    if (kind === draft.sourceKind) return;
    const value =
      kind === "screen"
        ? monitors[0]?.id.slice("screen:".length) ?? ""
        : windows[0]
          ? String(windows[0].pid)
          : "";
    const next = { ...draft, sourceKind: kind, sourceValue: value };
    setDraft(next);
    persistDraft(next);
    void applyLive({ sourceKind: kind, sourceValue: value }, kind === "screen" ? "monitor" : "window");
  }

  function chooseSourceValue(value: string) {
    const next = { ...draft, sourceValue: value };
    setDraft(next);
    persistDraft(next);
    void applyLive({ sourceValue: value }, "source");
  }

  function chooseAudio(value: RecordingAudio) {
    if (draft.mode === "audio" && value === "none") return;
    patchLive("audio", value, `audio ${value}`);
  }

  const bitrate = useMemo(
    () => estimateBitrateMbps(draft.resolution, draft.fps, draft.quality),
    [draft.resolution, draft.fps, draft.quality]
  );

  const hasLoopback = useMemo(() => devices.some((d) => LOOPBACK_RE.test(d)), [devices]);
  const audioRequiredButMissing = draft.mode === "audio" && draft.audio === "none";
  const canStart =
    !busy && !audioRequiredButMissing && (!needsMic(draft.audio) || devices.length > 0);
  const canToggle = isRecording || canStart;

  function showToast(msg: string) {
    setToast(msg);
    setTimeout(() => setToast(null), 3200);
  }

  async function handleStart() {
    if (!canStart) return;
    setBusy(true);
    const config = buildConfig();
    try {
      // Release the DirectShow device the idle meter is holding before the real
      // capture opens it.
      await stopAudioMeter().catch(() => {});
      levelRef.current = 0;
      await startRecording(config);
      onRecordingChange(true);
      onStatsChange({
        is_recording: true,
        duration_seconds: 0,
        size_mb: 0,
        level: 0,
        mode: draft.mode,
      });
    } catch (e) {
      showToast(`Failed to start: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function handleStop() {
    setBusy(true);
    try {
      const path = await stopRecording();
      const name = path.split(/[\\/]/).pop() ?? path;
      showToast(`Saved: ${name}`);
    } catch (e) {
      showToast(`Failed to stop: ${e}`);
    } finally {
      onRecordingChange(false);
      onStatsChange({
        is_recording: false,
        duration_seconds: 0,
        size_mb: 0,
        level: 0,
        mode: "",
      });
      setBusy(false);
    }
  }

  const audioKbps =
    draft.quality === "low" ? 128 : draft.quality === "medium" ? 160 : draft.quality === "ultra" ? 256 : 192;

  const statusLabel = isRecording ? "RECORDING" : busy ? "WORKING…" : "STANDBY";
  const summary =
    draft.mode === "audio"
      ? `Audio only · ${draft.quality} · AAC`
      : `${draft.resolution === "source" ? "Native" : draft.resolution} · ${draft.fps} fps · ${draft.quality}`;

  return (
    <div className="record-page fade-in">
      <div className={`rec-hero ${isRecording ? "rec-hero--live" : ""}`}>
        <div className="rec-hero-main">
          <div className="rec-btn-wrap">
            {isRecording && <span className="rec-btn-halo" />}
            <button
              className={`rec-round ${isRecording ? "rec-round--live" : ""}`}
              onClick={isRecording ? handleStop : handleStart}
              disabled={busy || !canToggle}
              aria-label={isRecording ? "Stop recording" : "Start recording"}
              title={isRecording ? "Stop and save the recording" : "Start recording"}
            >
              {isRecording ? (
                <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
                  <rect x="5" y="5" width="14" height="14" rx="2.5" />
                </svg>
              ) : (
                <span className="rec-round-dot" />
              )}
            </button>
            <span className={`rec-btn-caption ${isRecording ? "rec-btn-caption--live" : ""}`}>
              {isRecording ? "Stop" : "Start"}
            </span>
          </div>

          <div className="rec-status">
            <span className={`rec-status-label ${isRecording ? "rec-status-label--live" : ""}`}>
              {isRecording && <span className="rec-dot-pulse" />}
              {statusLabel}
            </span>
            <span className="rec-status-timer">
              {formatDuration(isRecording ? stats.duration_seconds : 0)}
            </span>
            <span className="rec-status-meta">
              {isRecording ? `${formatSize(stats.size_mb)} · ${summary}` : summary}
            </span>
          </div>
        </div>
        <canvas ref={canvasRef} className="rec-wave" />
      </div>

      {warning && (
        <div className="rec-alert" role="alert">
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M12 9v4" />
            <path d="M12 17h.01" />
            <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
          </svg>
          <span>{warning}</span>
        </div>
      )}

      <div className="rec-mode-row">
        <div className="rec-seg rec-seg--wide">
          <button
            className={`rec-seg-btn ${draft.mode === "video" ? "rec-seg-btn--active" : ""}`}
            onClick={() => chooseMode("video")}
            disabled={isRecording}
          >
            Video + Audio
          </button>
          <button
            className={`rec-seg-btn ${draft.mode === "audio" ? "rec-seg-btn--active" : ""}`}
            onClick={() => chooseMode("audio")}
            disabled={isRecording}
          >
            Audio Only
          </button>
        </div>
      </div>

      {draft.mode === "video" && (
        <div className="rec-card">
          {!isRecording && (
            previewOn ? (
              <div className="rec-preview">
                {previewUrl ? (
                  <img src={previewUrl} alt="Capture preview" />
                ) : (
                  <span className="rec-preview-empty">
                    {previewError ? `Preview unavailable: ${previewError}` : "Loading preview…"}
                  </span>
                )}
                <button
                  type="button"
                  className="rec-preview-toggle"
                  onClick={() => togglePreview(false)}
                  title="Hide preview"
                >
                  Hide preview
                </button>
              </div>
            ) : (
              <div className="rec-preview rec-preview--off">
                <button
                  type="button"
                  className="rec-preview-toggle rec-preview-toggle--center"
                  onClick={() => togglePreview(true)}
                >
                  Show preview
                </button>
              </div>
            )
          )}
          <div className="rec-row">
            <span className="rec-label">Capture source</span>
            <Segment
              value={draft.sourceKind}
              options={[
                { value: "screen" as const, label: "Monitor" },
                { value: "window" as const, label: "Window" },
              ]}
              onChange={(v) => chooseSourceKind(v)}
            />
          </div>
          <div className="rec-row rec-row--stack">
            <span className="rec-label">
              {draft.sourceKind === "screen" ? "Display" : "Window"}
              {isRecording && <span className="rec-hint rec-hint--live">switchable while recording</span>}
            </span>
            {draft.sourceKind === "screen" ? (
              <select
                className="rec-select"
                value={`screen:${draft.sourceValue}`}
                onChange={(e) => chooseSourceValue(e.target.value.slice("screen:".length))}
              >
                {monitors.length === 0 && <option value="screen:0">Display 1</option>}
                {monitors.map((monitor) => (
                  <option key={monitor.id} value={monitor.id}>
                    {monitor.name} · {monitor.resolution}
                  </option>
                ))}
              </select>
            ) : (
              <div className="rec-source-window">
                <select
                  className="rec-select"
                  value={draft.sourceValue}
                  onChange={(e) => chooseSourceValue(e.target.value)}
                >
                  {windows.length === 0 && <option value="">No windows found</option>}
                  {windows.map((win) => (
                    <option key={`${win.pid}:${win.title}`} value={String(win.pid)}>
                      {win.title}
                    </option>
                  ))}
                </select>
                <button
                  type="button"
                  className="rec-refresh"
                  onClick={() => void refreshWindows()}
                  title="Refresh window list"
                  aria-label="Refresh window list"
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M21 12a9 9 0 1 1-2.64-6.36" />
                    <path d="M21 3v6h-6" />
                  </svg>
                </button>
              </div>
            )}
          </div>
        </div>
      )}

      {draft.mode === "video" ? (
        <div className="rec-card">
          <div className="rec-row">
            <span className="rec-label">Resolution</span>
            <Segment
              value={draft.resolution}
              options={RESOLUTIONS}
              onChange={(v) => patch("resolution", v)}
              disabled={isRecording}
            />
          </div>
          <div className="rec-row">
            <span className="rec-label">Frame rate</span>
            <Segment
              value={draft.fps}
              options={FRAME_RATES.map((f) => ({ value: f, label: `${f}` }))}
              onChange={(v) => patch("fps", v)}
              disabled={isRecording}
            />
          </div>
          <div className="rec-row">
            <span className="rec-label">Quality</span>
            <Segment
              value={draft.quality}
              options={QUALITIES}
              onChange={(v) => patch("quality", v)}
              disabled={isRecording}
            />
          </div>
          <div className="rec-row rec-row--stack">
            <span className="rec-label">Encoder</span>
            <select
              className="rec-select"
              value={draft.codec}
              onChange={(e) => patch("codec", e.target.value)}
              disabled={isRecording}
            >
              {CODECS.map((c) => (
                <option key={c.value} value={c.value}>
                  {c.label}
                </option>
              ))}
            </select>
          </div>
          <div className="rec-estimate">
            ≈ {bitrate} Mbps · {draft.resolution === "source" ? "native" : draft.resolution} ·{" "}
            {draft.fps} fps
          </div>
        </div>
      ) : (
        <div className="rec-card">
          <div className="rec-row">
            <span className="rec-label">Quality</span>
            <Segment
              value={draft.quality}
              options={QUALITIES}
              onChange={(v) => patch("quality", v)}
              disabled={isRecording}
            />
          </div>
          <div className="rec-row">
            <span className="rec-label">Format</span>
            <span className="rec-static-value rec-static-value--inline">
              AAC · {audioKbps} kbps · 48 kHz
            </span>
          </div>
        </div>
      )}

      <div className="rec-card">
        <div className="rec-row">
          <span className="rec-label">Audio</span>
          <Segment
            value={draft.audio}
            options={AUDIO_OPTIONS}
            onChange={(v) => chooseAudio(v)}
          />
        </div>
        {needsMic(draft.audio) && (
          <div className="rec-row rec-row--stack">
            <span className="rec-label">
              Microphone
              <span className="rec-hint">
                {devices.length === 0 ? "not found" : draft.micDevice || "default"}
              </span>
            </span>
            <select
              className="rec-select"
              value={draft.micDevice}
              onChange={(e) => patchLive("micDevice", e.target.value, "microphone")}
              disabled={devices.length === 0}
            >
              {devices.length === 0 && <option value="">No devices</option>}
              {devices.map((device) => (
                <option key={device} value={device}>
                  {device}
                </option>
              ))}
            </select>
          </div>
        )}
        {needsSystem(draft.audio) && !hasLoopback && (
          <div className="rec-note">No loopback device found — enable Stereo Mix</div>
        )}
      </div>

      {toast && <div className="toast">{toast}</div>}

      <style>{`
        .record-page {
          display: flex;
          flex-direction: column;
          gap: 12px;
          padding: 16px 20px 20px;
        }
        .rec-alert {
          display: flex;
          align-items: flex-start;
          gap: 8px;
          padding: 9px 12px;
          border-radius: var(--radius-sm);
          font-size: 11.5px;
          line-height: 1.4;
          color: #fcd34d;
          background: rgba(245, 158, 11, 0.1);
          border: 1px solid rgba(245, 158, 11, 0.35);
        }
        .rec-alert svg {
          flex-shrink: 0;
          margin-top: 1px;
        }
        .rec-preview {
          position: relative;
          display: flex;
          align-items: center;
          justify-content: center;
          aspect-ratio: 16 / 9;
          border-radius: var(--radius-sm);
          overflow: hidden;
          background: #000;
          border: 1px solid var(--border);
        }
        .rec-preview img {
          display: block;
          width: 100%;
          height: 100%;
          object-fit: contain;
        }
        .rec-preview-empty {
          font-size: 11px;
          color: var(--text-dim);
          padding: 0 24px;
          text-align: center;
          overflow-wrap: anywhere;
        }
        .rec-preview--off {
          aspect-ratio: auto;
          min-height: 64px;
          background: var(--surface);
        }
        .rec-preview-toggle {
          position: absolute;
          top: 8px;
          right: 8px;
          padding: 4px 9px;
          border-radius: 999px;
          font-size: 11px;
          font-weight: 600;
          color: var(--text);
          background: rgba(0, 0, 0, 0.55);
          border: 1px solid rgba(255, 255, 255, 0.18);
          cursor: pointer;
          backdrop-filter: blur(4px);
          transition: border-color var(--transition), background var(--transition);
        }
        .rec-preview-toggle:hover {
          background: rgba(0, 0, 0, 0.75);
          border-color: rgba(124, 58, 237, 0.6);
        }
        .rec-preview-toggle--center {
          position: static;
        }
        .rec-hero {
          position: relative;
          display: flex;
          flex-direction: column;
          border-radius: var(--radius);
          background: linear-gradient(180deg, rgba(124,58,237,0.08), rgba(13,13,15,0.2));
          border: 1px solid var(--border);
          overflow: hidden;
          transition: border-color var(--transition), box-shadow var(--transition), background var(--transition);
        }
        .rec-hero--live {
          border-color: rgba(239,68,68,0.35);
          box-shadow: 0 0 28px rgba(239,68,68,0.12) inset;
          background: linear-gradient(180deg, rgba(239,68,68,0.09), rgba(13,13,15,0.2));
        }
        .rec-hero-main {
          display: flex;
          align-items: center;
          gap: 16px;
          padding: 14px 16px 6px;
        }
        .rec-btn-wrap {
          position: relative;
          width: 64px;
          display: flex;
          flex-direction: column;
          align-items: center;
          gap: 5px;
          flex-shrink: 0;
        }
        .rec-btn-halo {
          position: absolute;
          top: -4px;
          left: -4px;
          width: 72px;
          height: 72px;
          border-radius: 50%;
          border: 2px solid rgba(239,68,68,0.45);
          animation: pulse-ring 1.8s ease infinite;
          pointer-events: none;
        }
        .rec-round {
          position: relative;
          width: 64px;
          height: 64px;
          border-radius: 50%;
          display: flex;
          align-items: center;
          justify-content: center;
          background: linear-gradient(180deg, #2a2a33, #1b1b21);
          border: 2px solid rgba(239,68,68,0.55);
          color: #fff;
          box-shadow: 0 0 0 1px rgba(0,0,0,0.4), 0 6px 18px rgba(0,0,0,0.45), 0 0 20px rgba(239,68,68,0.18);
          cursor: pointer;
          transition: transform 0.15s ease, border-color 0.2s ease, box-shadow 0.2s ease;
        }
        .rec-round:hover:not(:disabled) {
          transform: scale(1.06);
          border-color: #ef4444;
          box-shadow: 0 0 0 1px rgba(0,0,0,0.4), 0 8px 22px rgba(0,0,0,0.5), 0 0 26px rgba(239,68,68,0.45);
        }
        .rec-round:active:not(:disabled) {
          transform: scale(0.97);
        }
        .rec-round-dot {
          width: 30px;
          height: 30px;
          border-radius: 50%;
          background: radial-gradient(circle at 35% 30%, #ff8a8a, #ef4444 55%, #b91c1c 100%);
          box-shadow: 0 0 16px rgba(239,68,68,0.7);
        }
        .rec-btn-caption {
          font-size: 9px;
          font-weight: 700;
          letter-spacing: 0.12em;
          text-transform: uppercase;
          color: var(--text-dim);
        }
        .rec-btn-caption--live {
          color: #fca5a5;
        }
        .rec-round--live {
          background: linear-gradient(180deg, #3a1114, #250b0d);
          border-color: #ef4444;
          color: #fff;
          box-shadow: 0 0 0 1px rgba(0,0,0,0.4), 0 0 26px rgba(239,68,68,0.55);
        }
        .rec-round--live:hover:not(:disabled) {
          border-color: #f87171;
        }
        .rec-status {
          display: flex;
          flex-direction: column;
          gap: 1px;
          min-width: 0;
        }
        .rec-status-label {
          display: inline-flex;
          align-items: center;
          gap: 6px;
          font-size: 10px;
          font-weight: 700;
          letter-spacing: 0.14em;
          color: var(--text-dim);
        }
        .rec-status-label--live {
          color: #fca5a5;
        }
        .rec-status-timer {
          font-size: 28px;
          font-weight: 700;
          letter-spacing: -0.03em;
          line-height: 1.05;
          font-variant-numeric: tabular-nums;
          color: var(--text);
        }
        .rec-status-meta {
          font-size: 11px;
          font-weight: 500;
          color: var(--text-dim);
          white-space: nowrap;
          overflow: hidden;
          text-overflow: ellipsis;
        }
        .rec-wave {
          display: block;
          width: 100%;
          height: 58px;
        }
        .rec-mode-row {
          display: flex;
          align-items: center;
          gap: 8px;
        }
        .rec-seg {
          display: flex;
          background: var(--surface-2);
          border: 1px solid var(--border);
          border-radius: var(--radius-pill);
          padding: 3px;
          gap: 2px;
          flex: 1;
        }
        .rec-seg--wide {
          width: 100%;
        }
        .rec-seg--disabled {
          opacity: 0.5;
        }
        .rec-seg-btn {
          flex: 1;
          padding: 5px 8px;
          border-radius: var(--radius-pill);
          font-size: 12px;
          font-weight: 500;
          color: var(--text-dim);
          background: transparent;
          white-space: nowrap;
        }
        .rec-seg-btn:hover:not(:disabled) {
          color: var(--text-muted);
        }
        .rec-seg-btn--active {
          background: var(--surface-3);
          color: var(--text);
        }
        .rec-seg-btn:disabled {
          cursor: default;
        }
        .rec-card {
          display: flex;
          flex-direction: column;
          background: var(--surface);
          border: 1px solid var(--border);
          border-radius: var(--radius);
          overflow: hidden;
        }
        .rec-row {
          display: flex;
          align-items: center;
          justify-content: space-between;
          gap: 10px;
          padding: 9px 12px;
          border-bottom: 1px solid var(--border);
        }
        .rec-row:last-child {
          border-bottom: none;
        }
        .rec-row--stack {
          flex-direction: column;
          align-items: stretch;
          gap: 6px;
        }
        .rec-label {
          display: flex;
          align-items: center;
          gap: 6px;
          font-size: 11px;
          font-weight: 600;
          text-transform: uppercase;
          letter-spacing: 0.06em;
          color: var(--text-dim);
        }
        .rec-hint {
          font-size: 10px;
          font-weight: 500;
          text-transform: none;
          letter-spacing: 0;
          color: var(--text-muted);
          max-width: 190px;
          overflow: hidden;
          text-overflow: ellipsis;
          white-space: nowrap;
        }
        .rec-hint--live {
          color: #fca5a5;
        }
        .rec-source-window {
          display: flex;
          align-items: center;
          gap: 6px;
        }
        .rec-source-window .rec-select {
          flex: 1;
          min-width: 0;
        }
        .rec-refresh {
          flex-shrink: 0;
          display: inline-flex;
          align-items: center;
          justify-content: center;
          width: 30px;
          height: 30px;
          border-radius: var(--radius-sm);
          color: var(--text-muted);
          background: var(--surface-2);
          border: 1px solid var(--border);
          cursor: pointer;
          transition: color var(--transition), border-color var(--transition);
        }
        .rec-refresh:hover {
          color: var(--text);
          border-color: rgba(124, 58, 237, 0.5);
        }
        .rec-row .rec-seg {
          flex: 0 1 auto;
          max-width: 74%;
        }
        .rec-row .rec-seg-btn {
          padding: 4px 7px;
          font-size: 11px;
        }
        .rec-select {
          width: 100%;
          padding: 6px 10px;
          border-radius: var(--radius-sm);
          font-size: 12px;
          cursor: pointer;
        }
        .rec-static-value {
          padding: 6px 10px;
          border-radius: var(--radius-sm);
          font-size: 12px;
          color: var(--text-muted);
          background: var(--surface-2);
          border: 1px solid var(--border);
        }
        .rec-static-value--inline {
          padding: 4px 10px;
          font-size: 11px;
          font-variant-numeric: tabular-nums;
        }
        .rec-estimate {
          padding: 6px 12px 8px;
          font-size: 10px;
          font-weight: 500;
          letter-spacing: 0.04em;
          color: var(--text-dim);
          text-align: right;
          font-variant-numeric: tabular-nums;
        }
        .rec-note {
          padding: 7px 12px;
          font-size: 10px;
          color: var(--text-muted);
          background: rgba(255,255,255,0.02);
          border-top: 1px solid var(--border);
        }
      `}</style>
    </div>
  );
}
