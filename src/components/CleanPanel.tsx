import { Sparkles, TriangleAlert, Zap } from "lucide-react";
import { BEST_SIZE_MB, BestModel, Model, formatDownload, formatStrength } from "../lib/clean";
import { Button, Field, ProgressBar, SectionLabel, Segmented, Slider } from "../ui";
import type { CleanStage } from "./Editor";

/** The editor's side panel: noise removal (Fast or Best) and its Strength. */
export function CleanPanel({
  stage,
  model,
  best,
  onModel,
  strength,
  onRemoveNoise,
  onCancel,
  onStrength,
  onStrengthCommit,
}: {
  stage: CleanStage;
  model: Model;
  best: BestModel | null;
  onModel: (m: Model) => void;
  strength: number;
  onRemoveNoise: () => void;
  onCancel: () => void;
  onStrength: (s: number) => void;
  onStrengthCommit: (s: number) => void;
}) {
  const busy = stage.kind === "downloading" || stage.kind === "running";
  const bestUnsupported = best !== null && !best.cpu.supported;
  return (
    <section className="inspector-section" aria-label="Noise removal">
        <SectionLabel>Noise removal</SectionLabel>
        <Field label="Model">
          <Segmented
            label="Model"
            value={model}
            onChange={onModel}
            disabled={busy}
            options={[
              { value: "fast", label: "Fast", icon: Zap, hint: "Quick, built in" },
              {
                value: "best",
                label: "Best",
                icon: Sparkles,
                hint: best?.downloaded ? "Clearest, slower" : `Clearest · ${BEST_SIZE_MB} MB download`,
                disabled: bestUnsupported,
              },
            ]}
          />
        </Field>
        {bestUnsupported && (
          <p className="inspector-hint">
            Best needs a processor with {best.cpu.missing.join(", ")}; this computer's doesn't have it.
          </p>
        )}

        {stage.kind === "none" && (
          <>
            <p className="inspector-text">
              {model === "fast"
                ? "Removes steady background noise: fans, air conditioning, hum, traffic rumble, keyboard clicks."
                : "Removes background noise more cleanly, including some background voices, and keeps the voice more natural. Takes longer."}{" "}
              Runs on this computer.
            </p>
            <Button variant="primary" icon={Sparkles} onClick={onRemoveNoise}>
              Remove noise
            </Button>
            {model === "best" && !best?.downloaded && (
              <p className="inspector-hint">
                The Best model is downloaded once ({BEST_SIZE_MB} MB) from KreoKit's own release; your
                recordings stay on this computer.
              </p>
            )}
          </>
        )}
        {stage.kind === "downloading" && (
          <>
            <p className="inspector-text">
              Downloading the Best model… {stage.progress ? formatDownload(stage.progress) : ""}
            </p>
            <ProgressBar
              value={stage.progress ? Math.round((stage.progress.downloaded_bytes / stage.progress.total_bytes) * 100) : null}
              label="Downloading the Best model"
            />
            <Button variant="secondary" size="sm" onClick={onCancel}>
              Cancel
            </Button>
          </>
        )}
        {stage.kind === "running" && (
          <>
            <p className="inspector-text">Removing noise… {Math.round(stage.fraction * 100)}%</p>
            <ProgressBar value={Math.round(stage.fraction * 100)} label="Removing noise" />
            <Button variant="secondary" size="sm" onClick={onCancel}>
              Cancel
            </Button>
          </>
        )}
        {stage.kind === "failed" && (
          <>
            <p className="inspector-error" role="alert">
              <TriangleAlert size={14} strokeWidth={2} aria-hidden /> {stage.message}
            </p>
            <Button variant="secondary" onClick={onRemoveNoise}>
              Try again
            </Button>
          </>
        )}
        {stage.kind === "done" && (
          <>
            <Field
              label={`Strength · ${formatStrength(strength)}`}
              hint="Lower keeps a little of the room for a natural sound; 100% removes all the noise the model finds."
            >
              <Slider
                label="Strength"
                value={Math.round(strength * 100)}
                min={0}
                max={100}
                onChange={(v) => onStrength(v / 100)}
                onCommit={(v) => onStrengthCommit(v / 100)}
                format={(v) => formatStrength(v / 100)}
              />
            </Field>
            <p className="inspector-hint">
              {model === "fast" ? "Fast" : "Best"} model ·{" "}
              {speed(stage.meta.realtime_factor)}
            </p>
          </>
        )}
    </section>
  );
}

/** "12× faster than real time" / "2.5 minutes per minute of sound". */
function speed(realtimeFactor: number): string {
  if (realtimeFactor < 1) return `${Math.max(1, Math.round(1 / Math.max(realtimeFactor, 1e-3)))}× faster than real time`;
  return `${realtimeFactor.toFixed(1)} minutes per minute of sound`;
}
