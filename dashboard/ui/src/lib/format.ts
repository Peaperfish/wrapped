// Number formatting by unit. Numbers render in tabular figures (see .num).
import type { UnitName } from "./types";

const pad = (n: number): string => String(n).padStart(2, "0");
const count = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });

export function fmtSecs(v: number): string {
  const s = Math.round(v);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${pad(s % 60)}s`;
  const h = Math.floor(s / 3600);
  return `${count.format(h)}h ${pad(Math.floor((s % 3600) / 60))}m`;
}

export function fmtBytes(v: number): string {
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let x = v;
  let i = 0;
  while (Math.abs(x) >= 1000 && i < units.length - 1) {
    x /= 1000;
    i++;
  }
  return i === 0 ? `${Math.round(x)} B` : `${x.toFixed(2)} ${units[i]}`;
}

export function fmtValue(v: number, unit: UnitName): string {
  switch (unit) {
    case "secs":
      return fmtSecs(v);
    case "bytes":
      return fmtBytes(v);
    case "count":
      return count.format(v);
  }
}

/** "+12.5%", "−3.0%", "0%", or "new" when there was nothing before. */
export function fmtPct(p: number | null): string {
  if (p === null) return "new";
  if (p === 0) return "0%";
  return `${p > 0 ? "+" : "−"}${Math.abs(p).toFixed(1)}%`;
}

const APP_NAMES: Record<string, string> = {
  "com.google.Chrome": "Chrome",
  "com.apple.finder": "Finder",
  "com.apple.Safari": "Safari",
  "com.mitchellh.ghostty": "Ghostty",
  "com.microsoft.Word": "Word",
  "com.tencent.xinWeChat": "WeChat",
};

/** A readable name for a bundle id ("com.google.Chrome" → "Chrome"): a few
 *  known ones, else the last part with its first letter capitalised. Other
 *  keys ("idle", "(unknown)") pass through. */
export function appName(bundleId: string): string {
  const known = APP_NAMES[bundleId];
  if (known) return known;
  const last = bundleId.split(".").pop() ?? bundleId;
  return bundleId.includes(".") ? last.charAt(0).toUpperCase() + last.slice(1) : bundleId;
}
