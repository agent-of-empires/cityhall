import { useState } from "react";
import { Checkbox, Input, Select } from "./ui";

export type VersionMode = "release" | "custom";

/// Which mode the field is in: the user's choice once they have made one, and
/// otherwise inferred from the current value.
///
/// Inferred per render rather than seeded once, because `value` and `versions`
/// both arrive from requests that land after mount. Seeding left a saved custom
/// tag sitting in the release dropdown with the checkbox unticked.
export function versionMode(value: string, versions: string[], override: VersionMode | null): VersionMode {
  if (override) return override;
  return value && !versions.includes(value) ? "custom" : "release";
}

export function VersionField({
  value,
  onChange,
  versions,
  latest,
  noneLabel,
  className,
}: {
  value: string;
  onChange: (next: string) => void;
  versions: string[];
  latest?: string;
  noneLabel: string;
  className?: string;
}) {
  // Null until the user touches the checkbox, so the mode follows the props
  // until then. A `useState` initializer runs once, on mount, and both `value`
  // and `versions` arrive from requests that land after it: a saved custom tag
  // would show up in the release dropdown with the box unticked. Once the user
  // has chosen, their choice wins and a save that round-trips the value must not
  // snap the box back.
  const [modeOverride, setModeOverride] = useState<VersionMode | null>(null);
  const mode = versionMode(value, versions, modeOverride);

  function toggleCustom(checked: boolean) {
    setModeOverride(checked ? "custom" : "release");
    // A release tag and a hand-typed tag are not interchangeable, so
    // switching clears rather than leaving a value behind that the other
    // mode does not account for.
    onChange("");
  }

  return (
    <div className={className}>
      <Checkbox checked={mode === "custom"} onChange={toggleCustom} label="custom version" />
      <div className="mt-1.5">
        {mode === "release" ? (
          versions.length > 0 ? (
            <Select value={value} onChange={(e) => onChange(e.target.value)}>
              <option value="">{noneLabel}</option>
              {/* A previously saved version can predate the discovered list. */}
              {value && !versions.includes(value) && <option value={value}>{value}</option>}
              {versions.map((v) => (
                <option key={v} value={v}>
                  {v}
                  {v === latest ? " (latest)" : ""}
                </option>
              ))}
            </Select>
          ) : (
            <Input value={value} onChange={(e) => onChange(e.target.value)} placeholder="v0.1.0" />
          )
        ) : (
          <Input value={value} onChange={(e) => onChange(e.target.value)} placeholder="main-20260804" />
        )}
      </div>
      {mode === "custom" && (
        <p className="mt-1 text-xs text-text-hint">
          Substituted into the image template; an image tagged with this version has to already exist or be buildable.
        </p>
      )}
    </div>
  );
}
