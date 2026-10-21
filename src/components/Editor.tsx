import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Clapperboard, Pause, Play, TriangleAlert } from "lucide-react";
import {
  CleanMeta,
  DEFAULT_STRENGTH,
  cancelClean,
  cleanFile,
  cleanStatus,
  cleanedWaveform,
  fileSettings,
  saveSettings,
} from "../lib/clean";
import { fileName, formatDuration, formatTime } from "../lib/files";
import { describeAudio } from "../lib/media";
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
import { OpenFile, useSession } from "../store/session";
import { Button, IconButton, Kbd, ProgressBar, Segmented, toast } from "../ui";
import { CleanPanel } from "./CleanPanel";
import { Timeline } from "./Timeline";
import { TopBar } from "./TopBar";
import "../styles/editor.css";

type Stage = { kind: "preparing"; fraction: number } | { kind: "ready" } | { kind: "failed"; message: string };

export type CleanStage =
  | { kind: "none" }
  | { kind: "running"; fraction: number }
  | { kind: "done"; meta: CleanMeta }
  | { kind: "failed"; message: string };

/** How often the playhead is read from the player while playing (drawn smoothly between). */
const POLL_MS = 100;

/** The open file: waveform, playback, noise removal with A/B and Strength. */
export function Editor({ file }: { file: OpenFile }) {
  const close = useSession((s) => s.close);
  const { info } = file;
  const [stage, setStage] = useState<Stage>({ kind: "preparing", fraction: 0 });
  const [clean, setClean] = useState<CleanStage>({ kind: "none" });
  const [peaks, setPeaks] = useState<Uint8Array | null>(null);
  const [cleanedPeaks, setCleanedPeaks] = useState<Uint8Array | null>(null);
  const [strength, setStrength] = useState(DEFAULT_STRENGTH);
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
      await invoke("prepare_preview", { path: file.path, channels: info.audio.channels, durationMs: info.duration_ms });
      const [wave, , settings, cleaned] = await Promise.all([
        invoke<ArrayBuffer>("waveform", { path: file.path }),
        playerOpen(file.path),
        fileSettings(file.path),
        cleanStatus(file.path),
      ]);
      let state = await playerStrength(settings.strength);
      if (cleaned) {
        setCleanedPeaks(await cleanedWaveform(file.path));
        state = await playerLoadCleaned(file.path);
      }
      if (!live) return;
      setPeaks(new Uint8Array(wave));
      setStrength(settings.strength);
      if (cleaned) setClean({ kind: "done", meta: cleaned });
      update(state);
      setStage({ kind: "ready" });
    })().catch((e) => live && setStage({ kind: "failed", message: String(e) }));
    return () => {
      live = false;
      void progress.then((un) => un());
      void invoke("cancel_preview");
      void cancelClean();
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

  const removeNoise = useCallback(async () => {
    setClean({ kind: "running", fraction: 0 });
    const progress = listen<{ path: string; fraction: number }>("clean-progress", (e) => {
      if (e.payload.path === file.path) setClean({ kind: "running", fraction: e.payload.fraction });
    });
    try {
      const meta = await cleanFile(file.path);
      setCleanedPeaks(await cleanedWaveform(file.path));
      update(await playerListen(true));
      setClean({ kind: "done", meta });
    } catch (e) {
      setClean(String(e) === "cancelled" ? { kind: "none" } : { kind: "failed", message: String(e) });
    } finally {
      void progress.then((un) => un());
    }
  }, [file.path, update]);

  const changeStrength = useCallback(
    (s: number) => {
      setStrength(s);
      void playerStrength(s).then(update);
    },
    [update],
  );
  const commitStrength = useCallback(
    (s: number) => void saveSettings(file.path, { strength: s }).catch((e) => toast(String(e), "error")),
    [file.path],
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
      } else if (e.key.toLowerCase() === "b" && hasCleaned) {
        e.preventDefault();
        listenTo(!listening);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [stage.kind, toggle, seek, currentMs, hasCleaned, listening, listenTo]);

  const durationMs = player?.duration_ms || info.duration_ms;
  return (
    <div className="screen editor">
      <TopBar title={<span className="editor-title">{fileName(file.path)}</span>} onHome={close} />
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
                {hasCleaned && (
                  <div className="transport-ab">
                    <Segmented
                      label="Listen to"
                      value={listening ? "cleaned" : "original"}
                      onChange={(v) => listenTo(v === "cleaned")}
                      options={[
                        { value: "original", label: "Original" },
                        { value: "cleaned", label: "Cleaned" },
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
          <CleanPanel
            stage={clean}
            strength={strength}
            onRemoveNoise={() => void removeNoise()}
            onCancel={() => void cancelClean()}
            onStrength={changeStrength}
            onStrengthCommit={commitStrength}
          />
        )}
      </div>
    </div>
  );
}
