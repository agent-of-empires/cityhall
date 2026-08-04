import { useState } from "react";
import { formatVersion, isGitVersion } from "../lib/versions";
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
  const [mode, setMode] = useState<"release" | "git">(() => (isGitVersion(value) ? "git" : "release"));
  const [gitRef, setGitRef] = useState("");

  function toggleGit(checked: boolean) {
    setMode(checked ? "git" : "release");
    // Neither mode can show the other's value, so switching clears rather than
    // leaving a value behind that the now-empty field does not account for.
    if (checked ? !isGitVersion(value) : true) onChange("");
  }

  return (
    <div className={className}>
      <label className="flex items-center gap-1.5 text-xs text-text-secondary">
        <input
          type="checkbox"
          checked={mode === "git"}
          onChange={(e) => toggleGit(e.target.checked)}
          className="h-4 w-4 accent-brand-500"
        />
        unreleased git ref (experimental)
      </label>
      <div className="mt-1.5">
        {mode === "release" ? (
          versions.length > 0 ? (
            <Select value={value} onChange={(e) => onChange(e.target.value)}>
              <option value="">{noneLabel}</option>
              {/* A previously saved version can predate the discovered list. */}
              {value && !versions.includes(value) && <option value={value}>{formatVersion(value)}</option>}
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
          <Input
            value={gitRef}
            onChange={(e) => {
              const ref = e.target.value;
              setGitRef(ref);
              onChange(ref.trim() ? `git:${ref.trim()}` : "");
            }}
            placeholder="main"
          />
        )}
      </div>
      {mode === "git" &&
        (isGitVersion(value) ? (
          <p className="mt-1 text-xs text-text-muted" title={value}>
            Currently {formatVersion(value)}.
          </p>
        ) : (
          <p className="mt-1 text-xs text-text-muted">
            Compiles aoe from source; first launch can take several minutes.
          </p>
        ))}
    </div>
  );
}
