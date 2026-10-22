import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { FolderOpen, TriangleAlert } from "lucide-react";
import {
  CANCELLED,
  Done,
  ExportOptions,
  ExportProgress,
  FORMAT_LABELS,
  Format,
  STAGE_LABELS,
  cancelExport,
  devExportFolder,
  exportFile,
  exportOptions,
  formatHint,
  freeName,
  revealExport,
  withExtension,
} from "../lib/exporting";
import { fileName } from "../lib/files";
import { formatBytes } from "../lib/media";
import { afterReadout } from "../lib/polish";
import { Button, Dialog, Field, ProgressBar, Select } from "../ui";

type State =
  | { kind: "loading" }
  | { kind: "ready" }
  | { kind: "conflict" }
  | { kind: "running"; progress: ExportProgress | null }
  | { kind: "done"; done: Done }
  | { kind: "failed"; message: string };

/** Export the open file exactly as it plays: format, where, then a checked file. */
export function ExportDialog({ path, onClose }: { path: string; onClose: () => void }) {
  const [options, setOptions] = useState<ExportOptions | null>(null);
  const [format, setFormat] = useState<Format>("wav");
  const [destination, setDestination] = useState("");
  const [state, setState] = useState<State>({ kind: "loading" });

  useEffect(() => {
    let live = true;
    Promise.all([exportOptions(path), devExportFolder()]).then(
      ([o, devFolder]) => {
        if (!live) return;
        setOptions(o);
        setFormat(o.format);
        setDestination(devFolder ? `${devFolder}/${fileName(o.destination)}` : o.destination);
        setState({ kind: "ready" });
      },
      (e) => live && setState({ kind: "failed", message: String(e) }),
    );
    return () => {
      live = false;
    };
  }, [path]);

  const formats: Format[] = options?.is_video ? ["video", "wav", "flac", "m4a", "mp3"] : ["wav", "flac", "m4a", "mp3"];
  const chooseFormat = (f: Format) => {
    setFormat(f);
    if (f === "video" && options) setDestination(destination.replace(/\.[^./\\]+$/, "") + options.destination.slice(options.destination.lastIndexOf(".")));
    else setDestination(withExtension(destination, f));
  };

  const change = async () => {
    const picked = await save({ title: "Export as", defaultPath: destination });
    if (picked) setDestination(picked);
  };

  const run = async (replace: boolean, to = destination) => {
    setState({ kind: "running", progress: null });
    const unlisten = await listen<ExportProgress>("export-progress", (e) => {
      if (e.payload.path === path) setState({ kind: "running", progress: e.payload });
    });
    try {
      const done = await exportFile(path, format, to, replace);
      setState({ kind: "done", done });
    } catch (e) {
      if (String(e) === CANCELLED) setState({ kind: "ready" });
      else if (/already exists/.test(String(e))) setState({ kind: "conflict" });
      else setState({ kind: "failed", message: String(e) });
    } finally {
      unlisten();
    }
  };

  const keepBoth = async () => {
    const free = await freeName(destination);
    setDestination(free);
    await run(false, free);
  };

  const running = state.kind === "running";
  const close = () => {
    if (running) void cancelExport();
    onClose();
  };

  let body: React.ReactNode = null;
  let footer: React.ReactNode = null;
  if (options?.nothing_to_do && state.kind === "ready") {
    body = (
      <p className="dialog-text">
        Nothing would change: the noise isn't removed, every Polish step is off and the level is kept as recorded.
        Turn something on, and export then.
      </p>
    );
    footer = (
      <Button variant="primary" onClick={onClose}>
        OK
      </Button>
    );
  } else if (state.kind === "ready" || state.kind === "loading") {
    body = (
      <div className="export-form">
        <Field label="Format">
          <Select<Format>
            label="Format"
            value={format}
            onChange={chooseFormat}
            options={formats.map((f) => ({
              value: f,
              label: FORMAT_LABELS[f],
              hint: formatHint(f, options),
            }))}
          />
        </Field>
        <Field label="Save as">
          <div className="export-destination">
            <span className="export-path" title={destination}>
              {fileName(destination)}
            </span>
            <Button variant="secondary" size="sm" icon={FolderOpen} onClick={() => void change()}>
              Change…
            </Button>
          </div>
        </Field>
        {options?.is_video && format !== "video" && (
          <p className="dialog-hint">Audio only, no picture: only the video's sound is saved, as you hear it.</p>
        )}
        <p className="dialog-hint">
          Exactly what you hear as {options?.cleaned ? "Cleaned" : "Polished"}: the loudness is measured over the whole
          file, and the written file is checked before it's saved.
        </p>
      </div>
    );
    footer = (
      <>
        <Button variant="secondary" onClick={onClose}>
          Cancel
        </Button>
        <Button
          variant="primary"
          disabled={state.kind === "loading" || (format === "mp3" && !options?.mp3)}
          onClick={() => void run(false)}
        >
          Export
        </Button>
      </>
    );
  } else if (state.kind === "conflict") {
    body = <p className="dialog-text">{fileName(destination)} already exists in that folder.</p>;
    footer = (
      <>
        <Button variant="secondary" onClick={() => setState({ kind: "ready" })}>
          Cancel
        </Button>
        <Button variant="secondary" onClick={() => void keepBoth()}>
          Keep both
        </Button>
        <Button variant="primary" onClick={() => void run(true)}>
          Replace
        </Button>
      </>
    );
  } else if (state.kind === "running") {
    const p = state.progress;
    body = (
      <div className="export-form">
        <p className="dialog-text">
          {p ? STAGE_LABELS[p.stage] : "Starting"}… {Math.round((p?.fraction ?? 0) * 100)}%
        </p>
        <ProgressBar value={Math.round((p?.fraction ?? 0) * 100)} label="Exporting" />
      </div>
    );
    footer = (
      <Button variant="secondary" onClick={() => void cancelExport()}>
        Cancel
      </Button>
    );
  } else if (state.kind === "done") {
    const d = state.done;
    const r = afterReadout({
      params: { stages: { rumble: false, de_ess: false, compress: false }, pre_gain_db: 0, gain_db: 0, limit: true },
      input_lufs: null,
      output_lufs: d.lufs,
      output_true_peak_db: d.true_peak_db,
    });
    body = (
      <div className="export-form">
        <p className="dialog-text">
          Saved <strong>{fileName(d.path)}</strong> ({formatBytes(d.bytes)}).
        </p>
        <p className="dialog-hint">
          Checked: {r.lufs} · {r.peak}
        </p>
      </div>
    );
    footer = (
      <>
        <Button variant="secondary" icon={FolderOpen} onClick={() => void revealExport(d.path)}>
          Show in folder
        </Button>
        <Button variant="primary" onClick={onClose}>
          Done
        </Button>
      </>
    );
  } else if (state.kind === "failed") {
    body = (
      <p className="inspector-error" role="alert">
        <TriangleAlert size={14} strokeWidth={2} aria-hidden /> {state.message}
      </p>
    );
    footer = (
      <Button variant="secondary" onClick={onClose}>
        Close
      </Button>
    );
  }

  return (
    <Dialog title={state.kind === "done" ? "Exported" : "Export"} onClose={close} width={480} footer={footer}>
      {body}
    </Dialog>
  );
}
