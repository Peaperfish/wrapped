// Theme and accent: sets data-theme ("light" | "dark", System resolved) and
// data-accent on <html>; app.css does the rest. The choice is cached in
// localStorage so the first paint already has it (the setting itself lives in
// the database and arrives with meta()).
import { invoke } from "@tauri-apps/api/core";

export type ThemeChoice = "system" | "light" | "dark";
export const THEMES: { id: ThemeChoice; title: string }[] = [
  { id: "system", title: "System" },
  { id: "light", title: "Light" },
  { id: "dark", title: "Dark" },
];
export const ACCENTS = ["green", "blue", "purple", "orange", "pink"] as const;
export type Accent = (typeof ACCENTS)[number];

const CACHE_KEY = "wrapped.appearance";
const media = window.matchMedia("(prefers-color-scheme: dark)");
let theme: ThemeChoice = "system";

const asTheme = (v: unknown): ThemeChoice => (v === "light" || v === "dark" ? v : "system");
const asAccent = (v: unknown): Accent => (ACCENTS as readonly unknown[]).includes(v) ? (v as Accent) : "green";

function paint(accent: Accent) {
  const el = document.documentElement;
  el.dataset.theme = theme === "system" ? (media.matches ? "dark" : "light") : theme;
  el.dataset.accent = accent;
}

/** Apply a choice now (also tells the window, so the title bar matches). */
export function applyAppearance(t: unknown, a: unknown) {
  theme = asTheme(t);
  const accent = asAccent(a);
  paint(accent);
  try {
    localStorage.setItem(CACHE_KEY, JSON.stringify({ theme, accent }));
  } catch {
    /* storage can be blocked */
  }
  // The window's theme (title bar); null = follow macOS live.
  invoke("set_window_theme", { theme: theme === "system" ? null : theme }).catch((e) =>
    console.warn("set_window_theme failed:", e),
  );
}

/** Before the first paint: the cached choice, and follow macOS live. */
export function initAppearance() {
  let saved: { theme?: unknown; accent?: unknown } = {};
  try {
    saved = JSON.parse(localStorage.getItem(CACHE_KEY) ?? "{}") ?? {};
  } catch {
    /* ignore */
  }
  theme = asTheme(saved.theme);
  paint(asAccent(saved.accent));
  media.addEventListener("change", () => {
    if (theme === "system") paint(asAccent(document.documentElement.dataset.accent));
  });
}
