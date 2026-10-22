import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Clapperboard, Download, Pause, Play, Trash2, TriangleAlert } from "lucide-react";
import {
  BestModel,
  CleanMeta,
  DEFAULT_SETTINGS,
  DownloadProgress,
  FileSettings,
  Model,
  bestModel,
  cancelClean,
  cancelModelDownload,
  cleanFile,
  cleanStatus,
  cleanedWaveform,
  downloadBestModel,
  fileSettings,
  saveSettings,
  strengthOf,
  withStrength,
} from "../lib/clean";
import { Analysis, formatFloor, formatLufs, formatTruePeak, notes } from "../lib/analysis";
import { fileName, formatDuration, formatTime } from "../lib/files";
import { describeAudio, removeRecent, removedMessage } from "../lib/media";
import {
  PlayerState,
  estimate,
  playerClose,
  playerListen,
  playerLoadCleaned,
  playerOpen,
  playerPause,
  playerPlay,
  playerSeek,
  playerState,
  playerStrength,
} from "../lib/player";
import { Measured, PolishSettings, afterReadout, cancelPolish, estimatePolish, measurePolish } from "../lib/polish";
import { OpenFile, useSession } from "../store/session";
import { Button, IconButton, Kbd, ProgressBar, Segmented, toast } from "../ui";
import { CleanPanel } from "./CleanPanel";
import { ConfirmDialog, REMOVE_TEXT } from "./ConfirmDialog";
import { ExportDialog } from "./ExportDialog";
import { PolishPanel } from "./PolishPanel";
import { Timeline } from "./Timeline";
import { TopBar } from "./TopBar";
import "../styles/editor.css";

type Stage = { kind: "preparing"; fraction: number } | { kind: "ready" } | { kind: "failed"; message: string };

export type CleanStage =
  | { kind: "none" }
  | { kind: "downloading"; progress: DownloadProgress | null }
  | { kind: "running"; fraction: number }
  | { kind: "done"; meta: CleanMeta }
  | { kind: "failed"; message: string };

/** How often the playhead is read from the player while playing (drawn smoothly between). */
const POLL_MS = 100;

/** The open file: waveform, playback, noise removal (Fast or Best) with A/B and Strength. */
export function Editor({ file }: { file: OpenFile }) {
  const close = useSession((s) => s.close);
  const { info } = file;
  const [stage, setStage] = useState<Stage>({ kind: "preparing", fraction: 0 });
  const [clean, setClean] = useState<CleanStage>({ kind: "none" });
  const [peaks, setPeaks] = useState<Uint8Array | null>(null);
  const [cleanedPeaks, setCleanedPeaks] = useState<Uint8Array | null>(null);
  const [settings, setSettings] = useState<FileSettings>(DEFAULT_SETTINGS);
  const [best, setBest] = useState<BestModel | null>(null);
  const [analysis, setAnalysis] = useState<Analysis | null>(null);
  const [measured, setMeasured] = useState<Measured | null>(null);
  /** `measured` is the quick estimate, the exact measurement still running. */
  const [estimated, setEstimated] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [measuring, setMeasuring] = useState<number | null>(null);
  /** The Strength the loudness was measured for (the slider's value once released). */
  const [measureStrength, setMeasureStrength] = useState<number | null>(null);
  const model = settings.model;
  const strength = strengthOf(settings);
  const [player, setPlayer] = useState<PlayerState | null>(null);
  const [now, setNow] = useState(0);
  const receivedAt = useRef(0);

  const update = useCallback((state: PlayerState) => {
    receivedAt.current = performance.now();
    setNow(receivedAt.current);
    setPlayer(state);
    if (state.error) toast(state.error, "error");
  }, []);

  // Read the sound into the preview (or find it), then the waveform, the player, the
  // file's Strength and an earlier cleaned sound.
  useEffect(() => {
    let live = true;
    const progress = listen<{ path: string; fraction: number }>("preview-progress", (e) => {
      if (live && e.payload.path === file.path) setStage({ kind: "preparing", fraction: e.payload.fraction });
    });
    (async () => {
      const prepared = await invoke<{ analysis: Analysis }>("prepare_preview", {
        path: file.path,
        channels: info.audio.channels,
        durationMs: info.duration_ms,
      });
      const [wave, , saved, bestInfo] = await Promise.all([
        invoke<ArrayBuffer>("waveform", { path: file.path }),
        playerOpen(file.path),
        fileSettings(file.path),
        bestModel(),
      ]);
      const cleaned = await cleanStatus(file.path, saved.model);
      let state = await playerStrength(strengthOf(saved));
      const cleanedPeaks = cleaned ? await cleanedWaveform(file.path, saved.model) : null;
      if (cleaned) state = await playerLoadCleaned(file.path, saved.model);
      if (!live) return;
      setPeaks(new Uint8Array(wave));
      setAnalysis(prepared.analysis);
      setSettings(saved);
      setMeasureStrength(strengthOf(saved));
      setBest(bestInfo);
      setCleanedPeaks(cleanedPeaks);
      if (cleaned) setClean({ kind: "done", meta: cleaned });
      update(state);
      setStage({ kind: "ready" });
    })().catch((e) => live && setStage({ kind: "failed", message: String(e) }));
    return () => {
      live = false;
      void progress.then((un) => un());
      void invoke("cancel_preview");
      void cancelClean();
      void cancelModelDownload();
      void cancelPolish();
      void playerClose();
    };
  }, [file.path, info.audio.channels, info.duration_ms, update]);

  // While playing: poll the player, and redraw the playhead every frame in between.
  const playing = player?.playing ?? false;
  useEffect(() => {
    if (!playing) return;
    let frame = 0;
    const tick = () => {
      setNow(performance.now());
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    const poll = window.setInterval(() => void playerState().then(update), POLL_MS);
    return () => {
      cancelAnimationFrame(frame);
      window.clearInterval(poll);
    };
  }, [playing, update]);

  const save = useCallback(
    (next: FileSettings) => void saveSettings(file.path, next).catch((e) => toast(String(e), "error")),
    [file.path],
  );

  const removeNoise = useCallback(async () => {
    const progress = listen<{ path: string; fraction: number }>("clean-progress", (e) => {
      if (e.payload.path === file.path) setClean({ kind: "running", fraction: e.payload.fraction });
    });
    const download = listen<DownloadProgress>("model-progress", (e) =>
      setClean({ kind: "downloading", progress: e.payload }),
    );
    try {
      if (model === "best" && !best?.downloaded) {
        setClean({ kind: "downloading", progress: null });
        await downloadBestModel();
        setBest(await bestModel());
      }
      setClean({ kind: "running", fraction: 0 });
      const meta = await cleanFile(file.path, model);
      setCleanedPeaks(await cleanedWaveform(file.path, model));
      update(await playerListen(true));
      setClean({ kind: "done", meta });
    } catch (e) {
      setClean(String(e) === "cancelled" ? { kind: "none" } : { kind: "failed", message: String(e) });
    } finally {
      void progress.then((un) => un());
      void download.then((un) => un());
    }
  }, [file.path, model, best, update]);

  /** Fast or Best: that model's own cleaned sound (if made before) and Strength. */
  const chooseModel = useCallback(
    async (next: Model) => {
      if (next === model) return;
      const nextSettings = { ...settings, model: next };
      setSettings(nextSettings);
      save(nextSettings);
      const cleaned = await cleanStatus(file.path, next);
      setCleanedPeaks(cleaned ? await cleanedWaveform(file.path, next) : null);
      await playerStrength(strengthOf(nextSettings));
      setMeasureStrength(strengthOf(nextSettings));
      update(await playerLoadCleaned(file.path, next));
      setClean(cleaned ? { kind: "done", meta: cleaned } : { kind: "none" });
    },
    [file.path, model, settings, save, update],
  );

  const changeStrength = useCallback(
    (s: number) => {
      setSettings((current) => withStrength(current, current.model, s));
      void playerStrength(s).then(update);
    },
    [update],
  );
  const commitStrength = useCallback(
    (s: number) => {
      save(withStrength(settings, model, s));
      setMeasureStrength(s);
    },
    [save, settings, model],
  );

  const changePolish = useCallback(
    (polish: PolishSettings) => {
      const next = { ...settings, polish };
      setSettings(next);
      save(next);
    },
    [settings, save],
  );

  const toggle = useCallback(() => {
    if (!player) return;
    void (player.playing ? playerPause() : playerPlay()).then(update, (e) => toast(String(e), "error"));
  }, [player, update]);
  const listenTo = useCallback((cleaned: boolean) => void playerListen(cleaned).then(update), [update]);
  const seek = useCallback((ms: number) => void playerSeek(ms).then(update), [update]);
  const currentMs = player ? estimate(player, receivedAt.current, now) : 0;
  const hasCleaned = player?.has_cleaned ?? false;
  const listening = player?.listen_cleaned ?? false;
  const hasProcessed = hasCleaned || measured !== null;

  // Measure the loudness through Polish whenever what it depends on settles (400 ms after
  // the last change); the player gets the result.
  const busyCleaning = clean.kind === "running" || clean.kind === "downloading";
  const polish = settings.polish;
  useEffect(() => {
    if (stage.kind !== "ready" || busyCleaning || measureStrength === null) return;
    let live = true;
    const progress = listen<{ path: string; fraction: number }>("polish-progress", (e) => {
      if (live && e.payload.path === file.path) setMeasuring(e.payload.fraction);
    });
    const timer = window.setTimeout(async () => {
      const cleaned = hasCleaned ? model : null;
      setMeasuring(0);
      try {
        // The quick estimate first (≈), at once in the player too; then the exact one.
        const estimate = await estimatePolish(file.path, cleaned, measureStrength, polish);
        if (!live) return;
        setMeasured(estimate);
        setEstimated(true);
        const exact = await measurePolish(file.path, cleaned, measureStrength, polish);
        if (!live) return;
        setMeasured(exact);
        setEstimated(false);
        setMeasuring(null);
      } catch (e) {
        if (!live || String(e) === "cancelled") return;
        setMeasuring(null);
        toast(`Couldn't measure the loudness: ${e}`, "error");
      }
    }, 300);
    return () => {
      live = false;
      window.clearTimeout(timer);
      void progress.then((un) => un());
    };
  }, [stage.kind, busyCleaning, file.path, hasCleaned, model, measureStrength, polish]);

  // Space plays or pauses; ← / → move 5 seconds; B switches Original / Cleaned.
  useEffect(() => {
    if (stage.kind !== "ready") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if ((e.target as HTMLElement).closest("input, textarea, [role=menu], [role=dialog]")) return;
      if (e.key === " ") {
        e.preventDefault();
        toggle();
      } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        if ((e.target as HTMLElement).closest("[role=slider], [role=radiogroup]")) return;
        e.preventDefault();
        seek(Math.max(0, currentMs + (e.key === "ArrowLeft" ? -5_000 : 5_000)));
      } else if (e.key.toLowerCase() === "b" && hasProcessed) {
        e.preventDefault();
        listenTo(!listening);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [stage.kind, toggle, seek, currentMs, hasProcessed, listening, listenTo]);

  const durationMs = player?.duration_ms || info.duration_ms;
  return (
    <div className="screen editor">
      <TopBar
        title={<span className="editor-title">{fileName(file.path)}</span>}
        onHome={close}
        fileItems={[{ label: "Remove from Recent…", icon: Trash2, onSelect: () => setRemoving(true) }]}
      >
        {stage.kind === "ready" && (
          <Button
            variant={clean.kind === "none" ? "secondary" : "primary"}
            size="sm"
            icon={Download}
            disabled={clean.kind === "running" || clean.kind === "downloading"}
            onClick={() => {
              if (player?.playing) void playerPause().then(update);
              setExporting(true);
            }}
          >
            Export…
          </Button>
        )}
      </TopBar>
      {exporting && <ExportDialog path={file.path} onClose={() => setExporting(false)} />}
      {removing && (
        <ConfirmDialog
          title="Remove from Recent?"
          text={REMOVE_TEXT}
          confirmLabel="Remove"
          onClose={() => setRemoving(false)}
          onConfirm={() =>
            void removeRecent(file.path).then(
              (freed) => {
                toast(removedMessage(freed), "success");
                close();
              },
              (e) => toast(String(e), "error"),
            )
          }
        />
      )}
      <div className="editor-body">
        <main className="editor-main">
          <section className="editor-facts" aria-label="About this file">
            <span>{formatDuration(info.duration_ms)}</span>
            <span>{describeAudio(info.audio)}</span>
            {info.video && (
              <span className="editor-video-note">
                <Clapperboard size={14} strokeWidth={1.75} aria-hidden />
                Video {info.video.width}×{info.video.height}: the picture is kept as it is; only the sound changes.
              </span>
            )}
          </section>

          {analysis && (
            <section className="editor-readout" aria-label="Original sound">
              <span className="readout-label">Original</span>
              <span title="Integrated loudness (EBU R128)">{formatLufs(analysis.integrated_lufs)}</span>
              <span title="Highest true peak">peak {formatTruePeak(analysis.true_peak_db)}</span>
              <span title="The quiet between words">noise {formatFloor(analysis.noise_floor_db)}</span>
              {notes(analysis).map((n) => (
                <span key={n} className="readout-note">
                  {n}
                </span>
              ))}
            </section>
          )}
          {(measured || measuring !== null) && (
            <section className="editor-readout" aria-label="Result" aria-live="polite">
              <span className="readout-label">After</span>
              {measured && (
                <>
                  <span title={estimated ? "A quick estimate; the exact measurement is running" : undefined}>
                    {afterReadout(measured, estimated).lufs}
                  </span>
                  <span>{afterReadout(measured, estimated).peak}</span>
                </>
              )}
              {measuring !== null && (
                <span className="readout-progress">
                  {estimated ? "measuring exactly" : "measuring"}… {Math.round(measuring * 100)}%
                </span>
              )}
            </section>
          )}

          {stage.kind === "preparing" && (
            <section className="editor-status" aria-live="polite">
              <p>Reading the sound…</p>
              <ProgressBar value={Math.round(stage.fraction * 100)} label="Reading the sound" />
              <Button variant="secondary" size="sm" onClick={close}>
                Cancel
              </Button>
            </section>
          )}

          {stage.kind === "failed" && (
            <section className="editor-status" role="alert">
              <p className="editor-error">
                <TriangleAlert size={16} strokeWidth={2} aria-hidden /> This file can't be played: {stage.message}
              </p>
              <Button variant="secondary" size="sm" onClick={close}>
                Back to Home
              </Button>
            </section>
          )}

          {stage.kind === "ready" && (
            <>
              <Timeline
                durationMs={durationMs}
                peaks={peaks}
                cleaned={hasCleaned && cleanedPeaks ? { peaks: cleanedPeaks, strength, listening } : null}
                currentMs={currentMs}
                playing={playing}
                onSeek={seek}
              />
              <section className="transport" aria-label="Playback">
                <IconButton
                  icon={playing ? Pause : Play}
                  label={playing ? "Pause" : "Play"}
                  shortcut="Space"
                  variant="secondary"
                  onClick={toggle}
                />
                <span className="transport-time">
                  {formatTime(currentMs)} <span className="transport-total">/ {formatTime(durationMs)}</span>
                </span>
                {hasProcessed && (
                  <div className="transport-ab">
                    <Segmented
                      label="Listen to"
                      value={listening ? "cleaned" : "original"}
                      onChange={(v) => listenTo(v === "cleaned")}
                      options={[
                        { value: "original", label: "Original" },
                        { value: "cleaned", label: hasCleaned ? "Cleaned" : "Polished" },
                      ]}
                    />
                    <Kbd>B</Kbd>
                  </div>
                )}
                <span className="transport-hint">
                  <Kbd>Space</Kbd> play or pause · <Kbd>←</Kbd> <Kbd>→</Kbd> 5 seconds
                </span>
              </section>
            </>
          )}
        </main>

        {stage.kind === "ready" && (
          <aside className="inspector" aria-label="Noise removal and polish">
            <CleanPanel
              stage={clean}
              model={model}
              best={best}
              onModel={(m) => void chooseModel(m)}
              strength={strength}
              onRemoveNoise={() => void removeNoise()}
              onCancel={() => void (clean.kind === "downloading" ? cancelModelDownload() : cancelClean())}
              onStrength={changeStrength}
              onStrengthCommit={commitStrength}
            />
            <PolishPanel settings={settings.polish} onChange={changePolish} />
          </aside>
        )}
      </div>
    </div>
  );
}
