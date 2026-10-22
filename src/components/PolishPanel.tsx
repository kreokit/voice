import { DEFAULT_POLISH, PolishSettings, TARGETS, Target, targetHint } from "../lib/polish";
import { Field, NumberField, SectionLabel, Select, Switch } from "../ui";

/** The side panel's Polish section: rumble, de-ess, compression and the loudness target. */
export function PolishPanel({
  settings,
  onChange,
}: {
  settings: PolishSettings;
  onChange: (s: PolishSettings) => void;
}) {
  const s = settings ?? DEFAULT_POLISH;
  const stage = (key: keyof PolishSettings["stages"], value: boolean) =>
    onChange({ ...s, stages: { ...s.stages, [key]: value } });
  return (
    <section className="inspector-section" aria-label="Polish">
      <SectionLabel>Polish</SectionLabel>
      <div className="inspector-switches">
        <Switch label="Remove low rumble" checked={s.stages.rumble} onChange={(v) => stage("rumble", v)} />
        <Switch label="Soften harsh S sounds" checked={s.stages.de_ess} onChange={(v) => stage("de_ess", v)} />
        <Switch label="Even out the level" checked={s.stages.compress} onChange={(v) => stage("compress", v)} />
      </div>
      <Field label="Loudness" hint={targetHint(s)}>
        <Select<Target>
          label="Loudness"
          value={s.target}
          onChange={(target) => onChange({ ...s, target })}
          options={TARGETS.map((t) => ({ value: t.value, label: t.label, hint: t.hint || undefined }))}
        />
      </Field>
      {s.target === "custom" && (
        <Field label="Target" inline>
          <NumberField
            label="Target loudness"
            value={s.custom_lufs}
            min={-30}
            max={-9}
            step={0.5}
            unit="LUFS"
            onChange={(custom_lufs) => onChange({ ...s, custom_lufs })}
          />
        </Field>
      )}
    </section>
  );
}
