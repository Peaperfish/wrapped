// Theme, palette and accent: sets data-theme ("light" | "dark", System resolved),
// data-palette and data-accent on <html>; app.css does the rest. Choices are
// cached in localStorage so the first paint already has them (theme and accent
// also live in the database and arrive with meta()).
import { invoke } from "@tauri-apps/api/core";

export type ThemeChoice = "system" | "light" | "dark";
export const THEMES: { id: ThemeChoice; title: string }[] = [
  { id: "system", title: "System" },
  { id: "light", title: "Light" },
  { id: "dark", title: "Dark" },
];
export type PaletteChoice = "graphite" | "warm-paper" | "forest";
export const PALETTES: { id: PaletteChoice; title: string }[] = [
  { id: "graphite", title: "Graphite" },
  { id: "warm-paper", title: "Warm Paper" },
  { id: "forest", title: "Forest" },
];
export const ACCENTS = ["green", "blue", "purple", "orange", "pink"] as const;
export type Accent = (typeof ACCENTS)[number];

const CACHE_KEY = "wrapped.appearance";
const PALETTE_KEY = "wrapped.palette";
const media = window.matchMedia("(prefers-color-scheme: dark)");
let theme: ThemeChoice = "system";
let palette: PaletteChoice = "graphite";

const asTheme = (v: unknown): ThemeChoice => (v === "light" || v === "dark" ? v : "system");
const asPalette = (v: unknown): PaletteChoice =>
  v === "warm-paper" || v === "forest" ? v : "graphite";
const asAccent = (v: unknown): Accent =>
  (ACCENTS as readonly unknown[]).includes(v) ? (v as Accent) : "green";

function readCachedPalette(): PaletteChoice {
  try {
    const direct = localStorage.getItem(PALETTE_KEY);
    if (direct === "graphite" || direct === "warm-paper" || direct === "forest") return direct;
    const saved = JSON.parse(localStorage.getItem(CACHE_KEY) ?? "{}") ?? {};
    return asPalette(saved.palette);
  } catch {
    return "graphite";
  }
}

export function getPalette(): PaletteChoice {
  return palette;
}

function paint(accent: Accent) {
  const el = document.documentElement;
  el.dataset.theme = theme === "system" ? (media.matches ? "dark" : "light") : theme;
  el.dataset.palette = palette;
  el.dataset.accent = accent;
}

/** Tell the window (title bar, background colour); null = follow macOS live. */
function syncWindow() {
  invoke("set_window_theme", {
    theme: theme === "system" ? null : theme,
    palette,
  }).catch((e) => console.warn("set_window_theme failed:", e));
}

/** Apply a choice now (also tells the window, so the title bar matches). */
export function applyAppearance(t: unknown, a: unknown, p?: unknown) {
  theme = asTheme(t);
  const accent = asAccent(a);
  if (p !== undefined) palette = asPalette(p);
  paint(accent);
  try {
    localStorage.setItem(CACHE_KEY, JSON.stringify({ theme, accent, palette }));
    localStorage.setItem(PALETTE_KEY, palette);
  } catch {
    /* storage can be blocked */
  }
  syncWindow();
}

export function applyPalette(p: unknown) {
  applyAppearance(theme, document.documentElement.dataset.accent, p);
}

/** Before the first paint: the cached choice, and follow macOS live. */
export function initAppearance() {
  let saved: { theme?: unknown; accent?: unknown; palette?: unknown } = {};
  try {
    saved = JSON.parse(localStorage.getItem(CACHE_KEY) ?? "{}") ?? {};
  } catch {
    /* ignore */
  }
  theme = asTheme(saved.theme);
  palette = readCachedPalette();
  paint(asAccent(saved.accent));
  syncWindow(); // the window is hidden until the first frame, so no flash
  media.addEventListener("change", () => {
    if (theme !== "system") return;
    paint(asAccent(document.documentElement.dataset.accent));
    syncWindow();
  });
}
