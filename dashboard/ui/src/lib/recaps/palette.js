// Recap slide colours, derived from the current accent. Plain JavaScript so
// scripts/check-contrast.mjs can import the same code the player uses.
//
// Each slide kind turns the accent's hue by a fixed amount, so a recap walks
// around the accent's neighbourhood (and the complement for AI). Text and
// highlight colours are then darkened (light theme) or lightened (dark theme)
// until they reach MIN_CONTRAST against both background stops.

/** Text, secondary text and highlights must reach this on the slide background. */
export const MIN_CONTRAST = 4.6;

/** Opacity of the glow at its centre (slides.ts draws it with this). */
export const GLOW_ALPHA = 0.55;

/** `top` at `alpha` over `under` (both "#rrggbb"). */
export function mix(top, alpha, under) {
  const a = hexRgb(top);
  const b = hexRgb(under);
  return rgbHex(a.map((c, i) => Math.round(c * alpha + b[i] * (1 - alpha))));
}

/** Hue turn per slide kind, in degrees. */
export const HUE_SHIFT = {
  intro: 0,
  screen_time: 28,
  top_apps: -28,
  time_of_day: 56,
  busiest_day: -56,
  records: 84,
  git: -84,
  photos: 140,
  ai: 180,
  disk: 112,
  summary: 0,
};

/** "#rrggbb" → [r, g, b] (0-255). */
export function hexRgb(hex) {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return [128, 128, 128];
  return [0, 2, 4].map((i) => parseInt(m[1].slice(i, i + 2), 16));
}

function rgbHsl([r, g, b]) {
  r /= 255;
  g /= 255;
  b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h;
  if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return [h * 60, s, l];
}

function hslRgb([h, s, l]) {
  h = ((h % 360) + 360) % 360;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  const [r, g, b] =
    h < 60 ? [c, x, 0] : h < 120 ? [x, c, 0] : h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : h < 300 ? [x, 0, c] : [c, 0, x];
  return [r, g, b].map((v) => Math.round((v + m) * 255));
}

export function rgbHex(rgb) {
  return "#" + rgb.map((v) => Math.max(0, Math.min(255, v)).toString(16).padStart(2, "0")).join("");
}

function lum(rgb) {
  const [r, g, b] = rgb.map((c) => {
    c /= 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio of two "#rrggbb" colours. */
export function contrast(a, b) {
  const [x, y] = [lum(hexRgb(a)), lum(hexRgb(b))].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

/** Move the lightness of `hsl` away from the backgrounds until it reaches
 *  MIN_CONTRAST against every one of them. */
function readable(hsl, bgs, dark) {
  let [h, s, l] = hsl;
  for (let i = 0; i < 60; i++) {
    const hex = rgbHex(hslRgb([h, s, l]));
    if (bgs.every((bg) => contrast(hex, bg) >= MIN_CONTRAST)) return hex;
    l = dark ? Math.min(1, l + 0.02) : Math.max(0, l - 0.02);
  }
  return dark ? "#ffffff" : "#000000";
}

/**
 * The colours of one slide.
 * @param {string} accentHex the theme's --accent ("#rrggbb")
 * @param {"dark" | "light"} theme
 * @param {string} kind a slide kind (see HUE_SHIFT)
 */
export function slidePalette(accentHex, theme, kind) {
  const [h0, s0] = rgbHsl(hexRgb(accentHex));
  const h = h0 + (HUE_SHIFT[kind] ?? 0);
  const s = Math.max(0.45, Math.min(0.9, s0));
  const dark = theme !== "light";
  const bg1 = rgbHex(hslRgb(dark ? [h, s * 0.55, 0.12] : [h, s * 0.75, 0.94]));
  const bg2 = rgbHex(hslRgb(dark ? [h + 30, s * 0.6, 0.2] : [h + 30, s * 0.7, 0.86]));
  const glow = rgbHex(hslRgb(dark ? [h - 20, s, 0.3] : [h - 20, s, 0.78]));
  // Text may sit on either gradient stop or on the glow drawn over them.
  const bgs = [bg1, bg2, mix(glow, GLOW_ALPHA, bg1), mix(glow, GLOW_ALPHA, bg2)];
  const ink = readable(dark ? [h, 0.25, 0.96] : [h, 0.45, 0.1], bgs, dark);
  const dim = readable(dark ? [h, 0.22, 0.78] : [h, 0.3, 0.3], bgs, dark);
  const hi = readable(dark ? [h, Math.min(0.95, s + 0.1), 0.68] : [h, s, 0.32], bgs, dark);
  // Bar tracks and decoration: not text, so no contrast rule.
  const track = dark ? "rgba(255,255,255,0.12)" : "rgba(15,23,42,0.10)";
  return { bg1, bg2, ink, dim, hi, track, glow, bgs };
}

/** Every slide kind's palette for a theme and accent, for the contrast check. */
export function allPalettes(accentHex, theme) {
  return Object.keys(HUE_SHIFT).map((kind) => ({ kind, ...slidePalette(accentHex, theme, kind) }));
}
