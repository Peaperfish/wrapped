// Number formatting for the Now cards. Values and units are separate pieces so
// the cards can give each a fixed width (no jumping when digits change).
const GIB = 1024 ** 3;
const MIB = 1024 ** 2;

export const DASH = "—";

/** "37" for a percentage; "—" for none. */
export function pct(v: number | null | undefined): string {
  return v == null ? DASH : String(Math.round(v));
}

/** [value, unit] of a rate in bytes per second, auto-scaled and decimal
 *  (1000-based, as the CLI and Activity Monitor): kB/s, MB/s, GB/s. */
export function rate(v: number | null | undefined): [string, string] {
  if (v == null) return [DASH, "kB/s"];
  const units = ["kB/s", "MB/s", "GB/s"];
  let x = v / 1000;
  let i = 0;
  while (x >= 1000 && i < units.length - 1) {
    x /= 1000;
    i++;
  }
  return [x < 10 ? x.toFixed(2) : x < 100 ? x.toFixed(1) : x.toFixed(0), units[i]!];
}

/** Bytes in 1024-based GB (as Activity Monitor shows memory), or MB when small. */
export function gb(v: number | null | undefined): string {
  if (v == null) return DASH;
  return v >= GIB ? `${(v / GIB).toFixed(1)} GB` : `${Math.round(v / MIB)} MB`;
}

export function celsius(v: number | null | undefined): string {
  return v == null ? DASH : `${Math.round(v)}°C`;
}
