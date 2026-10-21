import { Sparkles, TriangleAlert } from "lucide-react";
import { formatStrength } from "../lib/clean";
import { Button, Field, ProgressBar, SectionLabel, Slider } from "../ui";
import type { CleanStage } from "./Editor";

/** The editor's side panel: noise removal (Fast, RNNoise) and its Strength. */
export function CleanPanel({
  stage,
  strength,
  onRemoveNoise,
  onCancel,
  onStrength,
  onStrengthCommit,
}: {
  stage: CleanStage;
  strength: number;
  onRemoveNoise: () => void;
  onCancel: () => void;
  onStrength: (s: number) => void;
  onStrengthCommit: (s: number) => void;
}) {
  return (
    <aside className="inspector" aria-label="Noise removal">
      <section className="inspector-section">
        <SectionLabel>Noise removal</SectionLabel>
        {stage.kind === "none" && (
          <>
            <p className="inspector-text">
              Removes steady background noise: fans, air conditioning, hum, traffic rumble, keyboard clicks.
              Runs on this computer.
            </p>
            <Button variant="primary" icon={Sparkles} onClick={onRemoveNoise}>
              Remove noise
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
              Fast model · {Math.max(1, Math.round(1 / Math.max(stage.meta.realtime_factor, 1e-3)))}× faster than real time
            </p>
          </>
        )}
      </section>
    </aside>
  );
}
