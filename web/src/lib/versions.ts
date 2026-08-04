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
