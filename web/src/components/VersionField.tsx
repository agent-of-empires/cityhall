import { useState } from "react";
import { Input, Select } from "./ui";

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
  // Initialized from value, but the user can toggle freely afterward; it must
  // not snap back to "release" just because a save round-trips the value.
  const [mode, setMode] = useState<"release" | "custom">(() =>
    value && !versions.includes(value) ? "custom" : "release",
  );

  function toggleCustom(checked: boolean) {
    setMode(checked ? "custom" : "release");
    // A release tag and a hand-typed tag are not interchangeable, so
    // switching clears rather than leaving a value behind that the other
    // mode does not account for.
    onChange("");
  }

  return (
    <div className={className}>
      <label className="flex items-center gap-1.5 text-xs text-text-secondary">
        <input
          type="checkbox"
          checked={mode === "custom"}
          onChange={(e) => toggleCustom(e.target.checked)}
          className="h-4 w-4 accent-brand-500"
        />
        custom version
      </label>
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
        <p className="mt-1 text-xs text-text-muted">
          Substituted into the image template; an image tagged with this version has to already exist or be buildable.
        </p>
      )}
    </div>
  );
}
