#!/usr/bin/env node
// Contrast check for the dashboard themes: reads the variables from
// dashboard/ui/src/app.css, and for every theme x accent measures text, muted
// text and the accent on the card surface, plus the text on
// accent buttons. Needs 4.5:1 for body text, muted text (held to 4.5, the ask
// was 3), the accent and the button text. Faint text is decoration and only
// reported. Exits 1 on any failure.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const css = readFileSync(
  fileURLToPath(new URL("../dashboard/ui/src/app.css", import.meta.url)),
  "utf8",
);
const blocks = [...css.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, sel, body]) => ({
  sel: sel.trim().replace(/\s+/g, " "),
  vars: Object.fromEntries([...body.matchAll(/(--[\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()])),
}));
const get = (sel) => Object.assign({}, ...blocks.filter((b) => b.sel === sel).map((b) => b.vars));

const accents = ["green", "blue", "purple", "orange", "pink"];
const themes = {
  dark: (a) => ({
    ...get(":root"),
    ...(a === "green" ? {} : get(`:root[data-accent="${a}"]`)),
  }),
  light: (a) => ({
    ...get(":root"),
    ...get(':root[data-theme="light"]'),
    ...(a === "green" ? {} : get(`:root[data-accent="${a}"]`)),
    ...(a === "green" ? {} : get(`:root[data-theme="light"][data-accent="${a}"]`)),
  }),
};

function parse(v, vars) {
  v = v.trim().replace(/var\((--[\w-]+)\)/g, (_, n) => vars[n]);
  let m;
  if ((m = v.match(/^#([0-9a-f]{6})$/i)))
    return { rgb: [0, 2, 4].map((i) => parseInt(m[1].slice(i, i + 2), 16)), a: 1 };
  if ((m = v.match(/^rgba?\(([^)]+)\)$/))) {
    const p = m[1].split(",").map((x) => parseFloat(x));
    return { rgb: p.slice(0, 3), a: p[3] ?? 1 };
  }
  throw new Error(`cannot parse ${v}`);
}
const over = (top, under) => ({
  rgb: top.rgb.map((c, i) => c * top.a + under.rgb[i] * (1 - top.a)),
  a: 1,
});
const lum = ({ rgb }) => {
  const [r, g, b] = rgb.map((c) => {
    c /= 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (fg, bg) => {
  const [a, b] = [lum(fg), lum(bg)].sort((x, y) => y - x);
  return (a + 0.05) / (b + 0.05);
};

let failed = false;
const lowest = {};
for (const [theme, build] of Object.entries(themes)) {
  for (const a of accents) {
    const vars = build(a);
    // The card sits on the page background (plus its accent glow).
    const page = over(parse(vars["--bg-glow-1"], vars), parse(vars["--bg"], vars));
    const card = over(parse(vars["--card-bg"], vars), page);
    const accent = parse(vars["--accent"], vars);
    const checks = [
      ["text", parse(vars["--text"], vars), card, 4.5],
      ["muted", parse(vars["--text-dim"], vars), card, 4.5],
      // Faint text is decoration (hints, placeholders): reported, not enforced.
      ["faint", parse(vars["--text-faint"], vars), card, 0],
      ["accent", accent, card, 4.5],
      // Text on a primary button (accent at 90% over the card).
      ["on-accent", parse(vars["--accent-ink"], vars), over({ ...accent, a: 0.9 }, card), 4.5],
      // Status colours as text on the card (memory pressure, errors) and on
      // the stronger button background (.danger buttons).
      ["warn", parse(vars["--warn"], vars), card, 4.5],
      ["danger", parse(vars["--danger"], vars), card, 4.5],
      ["danger-btn", parse(vars["--danger"], vars), over(parse(vars["--card-bg-strong"], vars), card), 4.5],
    ];
    for (const [name, fg, bg, min] of checks) {
      const r = ratio(fg, bg);
      const key = `${theme}/${name}`;
      if (!lowest[key] || r < lowest[key].r) lowest[key] = { r, a };
      if (r < min) {
        failed = true;
        console.log(`FAIL ${theme} ${a} ${name}: ${r.toFixed(2)} < ${min}`);
      }
    }
  }
}
for (const [k, { r, a }] of Object.entries(lowest))
  console.log(`${k.padEnd(14)} lowest ${r.toFixed(2)} (${a})`);
process.exit(failed ? 1 : 0);
