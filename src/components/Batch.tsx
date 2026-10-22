import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { AudioLines, Check, Clapperboard, Download, FolderOpen, Sparkles, Square, TriangleAlert, Zap } from "lucide-react";
import {
  BEST_SIZE_MB,
  BestModel,
  DEFAULT_STRENGTH,
  Model,
  bestModel,
  downloadBestModel,
  formatStrength,
} from "../lib/clean";
import {
  BatchResult,
  CANCELLED,
  ExportProgress,
  FORMAT_LABELS,
  Format,
  Plan,
  STAGE_LABELS,
  cancelExport,
  conflictText,
  devExportFolder,
  exportBatch,
  exportPlan,
  formatLeft,
  freeName,
  revealExport,
  speedFor,
  timeLeft,
} from "../lib/exporting";
import { fileName, formatDuration } from "../lib/files";
import { formatBytes } from "../lib/media";
import { DEFAULT_POLISH, PolishSettings, afterReadout } from "../lib/polish";
import { OpenFile, useSession } from "../store/session";
import { Button, Dialog, Field, ProgressBar, SectionLabel, Segmented, Select, Slider, toast } from "../ui";
import { PolishPanel } from "./PolishPanel";
import { TopBar } from "./TopBar";
import "../styles/batch.css";

type Cleaning = "off" | Model;
type FormatChoice = "own" | Exclude<Format, "video">;

type Row =
  | { kind: "waiting" }
  | { kind: "running"; progress: ExportProgress | null }
  | { kind: "done"; result: BatchResult }
  | { kind: "failed"; message: string }
  | { kind: "skipped" };

type Step =
  | { kind: "idle" }
  | { kind: "downloading"; fraction: number | null }
  | { kind: "conflicts"; plan: Plan; folder: string }
  | { kind: "space"; plan: Plan }
  | { kind: "running"; plan: Plan; index: number; startedAt: number }
  | { kind: "summary"; results: BatchResult[]; folder: string };

/** Several files with the same settings, exported together (Export all). */
export function Batch({ files }: { files: OpenFile[] }) {
  const close = useSession((s) => s.close);
  const [cleaning, setCleaning] = useState<Cleaning>("fast");
  const [strength, setStrength] = useState(DEFAULT_STRENGTH.fast);
  const [polish, setPolish] = useState<PolishSettings>(DEFAULT_POLISH);
  const [format, setFormat] = useState<FormatChoice>("own");
  const [best, setBest] = useState<BestModel | null>(null);
  const [rows, setRows] = useState<Row[]>(() => files.map(() => ({ kind: "waiting" })));
  const [step, setStep] = useState<Step>({ kind: "idle" });
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    bestModel().then(setBest, () => {});
  }, []);

  const settings = useMemo(
    () => ({ cleaned: cleaning === "off" ? null : cleaning, strength, polish }),
    [cleaning, strength, polish],
  );
  const running = step.kind === "running";
  const busy = running || step.kind === "downloading";

  // Clock for "time left" while running.
  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [running]);

  const chooseCleaning = (c: Cleaning) => {
    setCleaning(c);
    if (c !== "off") setStrength(DEFAULT_STRENGTH[c]);
  };

  async function exportAll() {
    if (cleaning === "best" && !best?.downloaded) {
      setStep({ kind: "downloading", fraction: null });
      const un = await listen<{ downloaded_bytes: number; total_bytes: number }>("model-progress", (e) =>
        setStep({ kind: "downloading", fraction: e.payload.downloaded_bytes / e.payload.total_bytes }),
      );
      try {
        await downloadBestModel();
        setBest(await bestModel());
      } catch (e) {
        setStep({ kind: "idle" });
        if (String(e) !== CANCELLED) toast(`The Best model couldn't be downloaded: ${e}`, "error");
        return;
      } finally {
        un();
      }
      setStep({ kind: "idle" });
    }
    const folder =
      (await devExportFolder()) ?? (await open({ title: "Export all to…", directory: true, multiple: false }));
    if (typeof folder !== "string") return;
    let plan: Plan;
    try {
      plan = await exportPlan(
        files.map((f) => f.path),
        folder,
        format === "own" ? null : format,
        settings,
      );
    } catch (e) {
      toast(String(e), "error");
      return;
    }
    if (plan.nothing_to_do) {
      toast("Nothing would change: noise removal is off, every Polish step is off and the level is kept.", "error");
      return;
    }
    if (plan.free_bytes !== null && plan.total_bytes + plan.margin_bytes > plan.free_bytes) {
      setStep({ kind: "space", plan });
      return;
    }
    if (plan.items.some((i) => i.exists)) {
      setStep({ kind: "conflicts", plan, folder });
      return;
    }
    await start(plan, folder, "keep");
  }

  async function start(plan: Plan, folder: string, conflicts: "replace" | "keep") {
    const items = [];
    const next: Row[] = plan.items.map((i) => (i.error ? { kind: "failed", message: i.error } : { kind: "waiting" }));
    for (const item of plan.items) {
      if (item.error) continue;
      const destination = item.exists && conflicts === "keep" ? await freeName(item.destination) : item.destination;
      items.push({ path: item.path, destination, format: item.format, replace: item.exists && conflicts === "replace" });
    }
    setRows(next);
    setStep({ kind: "running", plan, index: 0, startedAt: Date.now() });
    const indexOf = new Map(plan.items.map((i, n) => [i.path, n]));
    const un = await listen<ExportProgress>("export-progress", (e) => {
      const row = indexOf.get(e.payload.path);
      if (row === undefined) return;
      setRows((rs) => rs.map((r, n) => (n === row ? { kind: "running", progress: e.payload } : r)));
      setStep((s) => (s.kind === "running" ? { ...s, index: row } : s));
    });
    try {
      const results = await exportBatch(items, settings);
      setRows((rs) =>
        rs.map((r, n) => {
          const result = results.find((x) => x.path === plan.items[n].path);
          if (!result) return r;
          if (result.done) return { kind: "done", result };
          if (result.error === CANCELLED) return { kind: "skipped" };
          return { kind: "failed", message: result.error ?? "failed" };
        }),
      );
      setStep({ kind: "summary", results, folder });
    } catch (e) {
      toast(String(e), "error");
      setStep({ kind: "idle" });
    } finally {
      un();
    }
  }

  // Time left: this file's remaining share and the files after it, at the measured speed.
  const left = (() => {
    if (step.kind !== "running") return null;
    const durations = step.plan.items.map((i) => (i.error ? 0 : i.duration_ms));
    const row = rows[step.index];
    const fraction = row?.kind === "running" ? (row.progress?.fraction ?? 0) : 0;
    return timeLeft(durations, step.index, fraction, speedFor(step.plan.speeds, settings.cleaned));
  })();
  void now;

  const doneCount = rows.filter((r) => r.kind === "done").length;
  const totalMs = files.reduce((s, f) => s + f.info.duration_ms, 0);

  return (
    <div className="screen batch">
      <TopBar title={<span className="editor-title">Batch · {files.length} files</span>} onHome={busy ? undefined : close}>
        {running ? (
          <Button variant="secondary" size="sm" icon={Square} onClick={() => void cancelExport()}>
            Stop
          </Button>
        ) : (
          <Button variant="primary" size="sm" icon={Download} disabled={busy} onClick={() => void exportAll()}>
            Export all…
          </Button>
        )}
      </TopBar>
      <div className="editor-body">
        <main className="batch-main">
          <p className="batch-summary">
            {files.length} files · {formatDuration(totalMs)} in all
            {running && left && (
              <>
                {" "}
                · {doneCount} of {files.length} done · <strong>{formatLeft(left.total)}</strong>
              </>
            )}
          </p>
          <ul className="batch-list">
            {files.map((f, n) => (
              <BatchRow key={f.path} file={f} row={rows[n]} fileLeft={step.kind === "running" && step.index === n ? left?.file : undefined} />
            ))}
          </ul>
        </main>
        <aside className="inspector" aria-label="Settings for all files">
          <section className="inspector-section">
            <SectionLabel>Noise removal</SectionLabel>
            <Segmented<Cleaning>
              label="Noise removal"
              value={cleaning}
              onChange={chooseCleaning}
              disabled={busy}
              options={[
                { value: "off", label: "Off" },
                { value: "fast", label: "Fast", icon: Zap },
                {
                  value: "best",
                  label: "Best",
                  icon: Sparkles,
                  disabled: best !== null && !best.cpu.supported,
                },
              ]}
            />
            {cleaning === "best" && !best?.downloaded && (
              <p className="inspector-hint">The Best model is downloaded once ({BEST_SIZE_MB} MB) when the export starts.</p>
            )}
            {step.kind === "downloading" && (
              <ProgressBar value={step.fraction === null ? null : Math.round(step.fraction * 100)} label="Downloading the Best model" />
            )}
            {cleaning !== "off" && (
              <Field label={`Strength · ${formatStrength(strength)}`}>
                <Slider
                  label="Strength"
                  value={Math.round(strength * 100)}
                  min={0}
                  max={100}
                  onChange={(v) => setStrength(v / 100)}
                  format={(v) => formatStrength(v / 100)}
                  disabled={busy}
                />
              </Field>
            )}
          </section>
          <PolishPanel settings={polish} onChange={setPolish} />
          <section className="inspector-section">
            <SectionLabel>Format</SectionLabel>
            <Field label="Audio files" hint="Videos stay videos: the picture is kept as it is, the sound replaced.">
              <Select<FormatChoice>
                label="Format"
                value={format}
                onChange={setFormat}
                options={[
                  { value: "own", label: "Each file's own kind" },
                  ...(["wav", "flac", "m4a", "mp3"] as const).map((f) => ({ value: f, label: FORMAT_LABELS[f] })),
                ]}
              />
            </Field>
          </section>
        </aside>
      </div>

      {step.kind === "conflicts" && (
        <Dialog
          title="Some files are already there"
          onClose={() => setStep({ kind: "idle" })}
          width={460}
          footer={
            <>
              <Button variant="secondary" onClick={() => setStep({ kind: "idle" })}>
                Cancel
              </Button>
              <Button variant="secondary" onClick={() => void start(step.plan, step.folder, "keep")}>
                Keep both
              </Button>
              <Button variant="primary" onClick={() => void start(step.plan, step.folder, "replace")}>
                Replace
              </Button>
            </>
          }
        >
          <p className="dialog-text">
            {conflictText(step.plan.items.filter((i) => i.exists).map((i) => fileName(i.destination)))}
          </p>
        </Dialog>
      )}
      {step.kind === "space" && (
        <Dialog
          title="Not enough free space"
          onClose={() => setStep({ kind: "idle" })}
          width={440}
          footer={
            <Button variant="primary" onClick={() => setStep({ kind: "idle" })}>
              OK
            </Button>
          }
        >
          <p className="dialog-text">
            The files need about {formatBytes(step.plan.total_bytes)} and {formatBytes(step.plan.free_bytes ?? 0)} is
            free; KreoKit keeps {formatBytes(step.plan.margin_bytes)} spare. Choose another folder or make room.
          </p>
        </Dialog>
      )}
      {step.kind === "summary" && (
        <Dialog
          title={summaryTitle(step.results)}
          onClose={() => setStep({ kind: "idle" })}
          width={480}
          footer={
            <>
              {step.results.some((r) => r.done) && (
                <Button
                  variant="secondary"
                  icon={FolderOpen}
                  onClick={() => void revealExport(step.results.find((r) => r.done)!.done!.path)}
                >
                  Show in folder
                </Button>
              )}
              <Button variant="primary" onClick={() => setStep({ kind: "idle" })}>
                Done
              </Button>
            </>
          }
        >
          <ul className="batch-results">
            {step.results.map((r) => (
              <li key={r.path}>
                <strong>{fileName(r.path)}</strong>:{" "}
                {r.done
                  ? `${afterReadout({ params: { stages: DEFAULT_POLISH.stages, pre_gain_db: 0, gain_db: 0, limit: true }, input_lufs: null, output_lufs: r.done.lufs, output_true_peak_db: r.done.true_peak_db }).lufs}`
                  : r.error === CANCELLED
                    ? "stopped"
                    : r.error}
              </li>
            ))}
          </ul>
        </Dialog>
      )}
    </div>
  );
}

function summaryTitle(results: BatchResult[]): string {
  const done = results.filter((r) => r.done).length;
  if (done === results.length) return `Exported all ${done} files`;
  return `Exported ${done} of ${results.length} files`;
}

function BatchRow({ file, row, fileLeft }: { file: OpenFile; row: Row; fileLeft?: number }) {
  const Icon = file.info.video ? Clapperboard : AudioLines;
  let status: React.ReactNode;
  switch (row.kind) {
    case "waiting":
      status = <span className="batch-status">Waiting</span>;
      break;
    case "running": {
      const p = row.progress;
      status = (
        <span className="batch-status running">
          {p ? STAGE_LABELS[p.stage] : "Starting"} · {Math.round((p?.fraction ?? 0) * 100)}%
          {fileLeft !== undefined && ` · ${formatLeft(fileLeft)}`}
        </span>
      );
      break;
    }
    case "done":
      status = (
        <span className="batch-status done">
          <Check size={14} strokeWidth={2} aria-hidden /> {fileName(row.result.done!.path)}
        </span>
      );
      break;
    case "failed":
      status = (
        <span className="batch-status failed" title={row.message}>
          <TriangleAlert size={14} strokeWidth={2} aria-hidden /> {row.message}
        </span>
      );
      break;
    case "skipped":
      status = <span className="batch-status">Stopped</span>;
      break;
  }
  return (
    <li className="batch-row">
      <span className="recent-icon">
        <Icon size={18} strokeWidth={1.75} aria-hidden />
      </span>
      <span className="recent-info">
        <span className="recent-name" title={file.path}>
          {fileName(file.path)}
        </span>
        <span className="recent-meta">
          {formatDuration(file.info.duration_ms)} · {file.info.video ? "Video" : "Audio"}
        </span>
      </span>
      {status}
      {row.kind === "running" && (
        <span className="batch-progress">
          <ProgressBar value={Math.round((row.progress?.fraction ?? 0) * 100)} label={`Exporting ${fileName(file.path)}`} />
        </span>
      )}
    </li>
  );
}
