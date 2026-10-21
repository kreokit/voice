import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Clapperboard, Pause, Play, TriangleAlert } from "lucide-react";
import { fileName, formatDuration, formatTime } from "../lib/files";
import { describeAudio } from "../lib/media";
import {
  PlayerState,
  estimate,
  playerClose,
  playerOpen,
  playerPause,
  playerPlay,
  playerSeek,
  playerState,
} from "../lib/player";
import { OpenFile, useSession } from "../store/session";
import { Button, IconButton, Kbd, ProgressBar, toast } from "../ui";
import { Timeline } from "./Timeline";
import { TopBar } from "./TopBar";
import "../styles/editor.css";

type Stage = { kind: "preparing"; fraction: number } | { kind: "ready" } | { kind: "failed"; message: string };

/** How often the playhead is read from the player while playing (drawn smoothly between). */
const POLL_MS = 100;

/** The open file: its waveform and playback. Cleaning arrives with the next milestones. */
export function Editor({ file }: { file: OpenFile }) {
  const close = useSession((s) => s.close);
  const { info } = file;
  const [stage, setStage] = useState<Stage>({ kind: "preparing", fraction: 0 });
  const [peaks, setPeaks] = useState<Uint8Array | null>(null);
  const [player, setPlayer] = useState<PlayerState | null>(null);
  const [now, setNow] = useState(0);
  const receivedAt = useRef(0);

  const update = useCallback((state: PlayerState) => {
    receivedAt.current = performance.now();
    setNow(receivedAt.current);
    setPlayer(state);
    if (state.error) toast(state.error, "error");
  }, []);

  // Decode the preview (or find it), then load the waveform and the player.
  useEffect(() => {
    let live = true;
    const progress = listen<{ path: string; fraction: number }>("preview-progress", (e) => {
      if (live && e.payload.path === file.path) setStage({ kind: "preparing", fraction: e.payload.fraction });
    });
    invoke("prepare_preview", { path: file.path, channels: info.audio.channels, durationMs: info.duration_ms })
      .then(() => Promise.all([invoke<ArrayBuffer>("waveform", { path: file.path }), playerOpen(file.path)]))
      .then(([wave, state]) => {
        if (!live) return;
        setPeaks(new Uint8Array(wave));
        update(state);
        setStage({ kind: "ready" });
      })
      .catch((e) => live && setStage({ kind: "failed", message: String(e) }));
    return () => {
      live = false;
      void progress.then((un) => un());
      void invoke("cancel_preview");
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

  const toggle = useCallback(() => {
    if (!player) return;
    void (player.playing ? playerPause() : playerPlay()).then(update, (e) => toast(String(e), "error"));
  }, [player, update]);
  const seek = useCallback((ms: number) => void playerSeek(ms).then(update), [update]);
  const currentMs = player ? estimate(player, receivedAt.current, now) : 0;

  // Space plays or pauses; ← / → move 5 seconds.
  useEffect(() => {
    if (stage.kind !== "ready") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if ((e.target as HTMLElement).closest("input, textarea, [role=menu], [role=dialog]")) return;
      if (e.key === " ") {
        e.preventDefault();
        toggle();
      } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        e.preventDefault();
        seek(Math.max(0, currentMs + (e.key === "ArrowLeft" ? -5_000 : 5_000)));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [stage.kind, toggle, seek, currentMs]);

  const durationMs = player?.duration_ms || info.duration_ms;
  return (
    <div className="screen editor">
      <TopBar title={<span className="editor-title">{fileName(file.path)}</span>} onHome={close} />
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
            <Timeline durationMs={durationMs} peaks={peaks} currentMs={currentMs} playing={playing} onSeek={seek} />
            <section className="transport" aria-label="Playback">
              <IconButton
                icon={playing ? Pause : Play}
                label={playing ? "Pause" : "Play"}
                shortcut="Space"
                variant="primary"
                onClick={toggle}
              />
              <span className="transport-time">
                {formatTime(currentMs)} <span className="transport-total">/ {formatTime(durationMs)}</span>
              </span>
              <span className="transport-hint">
                <Kbd>Space</Kbd> play or pause · <Kbd>←</Kbd> <Kbd>→</Kbd> 5 seconds
              </span>
            </section>
          </>
        )}
      </main>
    </div>
  );
}
