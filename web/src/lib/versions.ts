// A source build is stored/returned as "git-<40-char sha>", never a release
// tag, so it needs its own check before anything tries to treat it as one.
export function isGitVersion(version: string): boolean {
  return version.startsWith("git-");
}

// Version ordering by numeric components ("v1.10.0" > "v1.9.9"); mirrors the
// server's version_key. Tags without digits compare as empty and are never
// considered outdated.
export function versionKey(tag: string): number[] {
  return tag
    .split(/[^0-9]+/)
    .filter((s) => s.length > 0)
    .map((s) => Number(s));
}

export function isOlderVersion(tag: string, latest: string): boolean {
  // A source build's sha is not a release ordering; digits inside it would
  // otherwise be parsed as version components and compared as garbage.
  if (isGitVersion(tag) || isGitVersion(latest)) return false;
  const a = versionKey(tag);
  const b = versionKey(latest);
  if (a.length === 0 || b.length === 0) return false;
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] ?? 0;
    const y = b[i] ?? 0;
    if (x !== y) return x < y;
  }
  return false;
}
