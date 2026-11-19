// Recap slides as self-contained SVG. The player has landscape and portrait
// layouts and the PNG export uses the portrait layout without animations.
// Colours are baked in from the current theme and accent (see
// palette.js); text is measured with a canvas in the same font and shrunk or
// truncated with an ellipsis so it never leaves its box.
import { GLOW_ALPHA, slidePalette, type SlidePalette } from "./palette.js";
import { aiToolName, modelLabel } from "../aiNames";
import type { RecapJson, SlideJson } from "../types";

export const W = 1080;
export const H = 1920;
const LANDSCAPE_W = 1600;
const LANDSCAPE_H = 900;
const M = 96; // side margin
const CW = W - 2 * M; // content width
export const FONT = `-apple-system, BlinkMacSystemFont, "Helvetica Neue", Helvetica, Arial, sans-serif`;

export interface SlideCtx {
  recap: RecapJson;
  /** App id → display name. */
  names: Record<string, string>;
  theme: "dark" | "light";
  /** The theme's --accent, "#rrggbb". */
  accent: string;
}

// ---------- formatting (also used by the count-up) ----------

export type Fmt = "int" | "hm" | "tokens" | "gb" | "days";

const intFmt = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });

/** "32h 15m", "45m", "30s". */
export function hm(secs: number): string {
  const s = Math.round(secs);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return `${intFmt.format(h)}h ${String(m % 60).padStart(2, "0")}m`;
}

export function tokens(v: number): string {
  const a = Math.abs(v);
  if (a < 1000) return String(Math.round(v));
  if (a < 1e6) return `${(v / 1e3).toFixed(a < 1e4 ? 1 : 0)}K`;
  if (a < 1e9) return `${(v / 1e6).toFixed(a < 1e7 ? 2 : 1)}M`;
  return `${(v / 1e9).toFixed(2)}B`;
}

export function gb(bytes: number): string {
  const g = bytes / 1e9;
  if (g >= 1000) return `${(g / 1000).toFixed(g >= 10_000 ? 0 : 1)} TB`;
  if (g >= 100) return `${Math.round(g)} GB`;
  return `${g.toFixed(1)} GB`;
}

export function fmt(f: Fmt, v: number): string {
  switch (f) {
    case "int":
      return intFmt.format(Math.round(v));
    case "hm":
      return hm(v);
    case "tokens":
      return tokens(v);
    case "gb":
      return gb(v);
    case "days":
      return `${Math.round(v)} ${Math.round(v) === 1 ? "day" : "days"}`;
  }
}

const WEEKDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

function parseDay(s: string): { y: number; m: number; d: number } {
  const [y, m, d] = s.split("-").map(Number);
  return { y: y ?? 1970, m: m ?? 1, d: d ?? 1 };
}
const longDate = (s: string) => {
  const p = parseDay(s);
  return `${p.d} ${MONTHS[p.m - 1]} ${p.y}`;
};
const shortDate = (s: string) => {
  const p = parseDay(s);
  return `${p.d} ${MONTHS[p.m - 1]?.slice(0, 3)}`;
};
const clock = (min: number) => {
  const m = ((min % 1440) + 1440) % 1440;
  return `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
};
const hourLabel = (h: number) => `${String(h).padStart(2, "0")}:00`;

function changeText(pct: number | null, ctx: SlideCtx): string {
  const vs =
    ctx.recap.in_progress && ctx.recap.kind === "year"
      ? `same days of ${Number(ctx.recap.previous.from.slice(0, 4))}`
      : `last ${ctx.recap.kind}`;
  if (pct === null || !ctx.recap.comparable) return `— vs ${vs}`;
  if (Math.abs(pct) < 0.5) return `same as ${vs}`;
  const r = Math.abs(pct) >= 10 ? Math.round(Math.abs(pct)) : Math.abs(pct).toFixed(1);
  return `${pct > 0 ? "+" : "−"}${r}% vs ${vs}`;
}

// ---------- text measuring ----------

let mctx: CanvasRenderingContext2D | null = null;
function measure(text: string, size: number, weight: number): number {
  mctx ??= document.createElement("canvas").getContext("2d");
  if (!mctx) return text.length * size * 0.6;
  mctx.font = `${weight} ${size}px ${FONT}`;
  return mctx.measureText(text).width;
}

/** The largest size ≤ `max` (≥ `min`) at which `text` fits `width`; below
 *  `min` the text is cut with an ellipsis. Allow for differences between
 *  canvas text measurement and SVG glyph bounds. */
export function fit(text: string, weight: number, max: number, width: number, min = max * 0.45) {
  const w = width * 0.92;
  const at = measure(text, max, weight);
  if (at <= w) return { text, size: max };
  const size = Math.max(min, Math.floor((max * w) / at));
  if (measure(text, size, weight) <= w) return { text, size };
  return { text: ellipsize(text, weight, size, w), size };
}

function ellipsize(text: string, weight: number, size: number, width: number): string {
  const chars = [...text];
  let lo = 0;
  let hi = chars.length;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (measure(chars.slice(0, mid).join("").trimEnd() + "…", size, weight) <= width) lo = mid;
    else hi = mid - 1;
  }
  return chars.slice(0, lo).join("").trimEnd() + "…";
}

// ---------- SVG pieces ----------

const esc = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

interface TextOpts {
  size: number;
  weight?: number;
  fill: string;
  anchor?: "start" | "middle" | "end";
  /** Animation class and delay (ms). */
  a?: string;
  d?: number;
  /** Count up to `n`, formatted with `f`. */
  n?: number;
  f?: Fmt;
  spacing?: number;
  opacity?: number;
}

function text(x: number, y: number, s: string, o: TextOpts): string {
  const attrs = [
    `x="${x}"`,
    `y="${y}"`,
    `font-size="${o.size}"`,
    `font-weight="${o.weight ?? 700}"`,
    `fill="${o.fill}"`,
  ];
  if (o.anchor && o.anchor !== "start") attrs.push(`text-anchor="${o.anchor}"`);
  if (o.spacing) attrs.push(`letter-spacing="${o.spacing}"`);
  if (o.opacity !== undefined) attrs.push(`fill-opacity="${o.opacity}"`);
  if (o.n !== undefined && o.f) attrs.push(`data-n="${o.n}"`, `data-f="${o.f}"`);
  const t = `<text ${attrs.join(" ")}>${esc(s)}</text>`;
  return o.a ? anim(o.a, o.d ?? 0, t) : t;
}

/** Wrap in an animated group (the class only does something on screen). */
function anim(cls: string, delay: number, inner: string): string {
  return `<g class="${cls}" style="animation-delay:${delay}ms">${inner}</g>`;
}

/** A fitted line of text. */
function line(
  x: number,
  y: number,
  s: string,
  width: number,
  max: number,
  o: Omit<TextOpts, "size">,
  min?: number,
): string {
  const f = fit(s, o.weight ?? 700, max, width, min);
  return text(x, y, f.text, { ...o, size: f.size });
}

/** A big number that counts up; its final text decides the size. */
function bigNumber(
  x: number,
  y: number,
  value: number,
  f: Fmt,
  width: number,
  max: number,
  o: Omit<TextOpts, "size" | "n" | "f">,
): { svg: string; size: number } {
  const s = fmt(f, value);
  const fitted = fit(s, o.weight ?? 800, max, width, 40);
  return {
    svg: text(x, y, fitted.text, {
      ...o,
      weight: o.weight ?? 800,
      size: fitted.size,
      ...(fitted.text === s ? { n: value, f } : {}),
    }),
    size: fitted.size,
  };
}

function kicker(s: string, p: SlidePalette, y = 330): string {
  return text(M, y, s.toUpperCase(), { size: 40, weight: 700, fill: p.dim, spacing: 6, a: "a-fade", d: 0 });
}

function chip(x: number, y: number, s: string, p: SlidePalette, delay: number, width = CW): string {
  const f = fit(s, 600, 40, width - 60, 28);
  const w = Math.min(width, measure(f.text, f.size, 600) + 64);
  return anim(
    "a-fade",
    delay,
    `<rect x="${x}" y="${y}" width="${w.toFixed(0)}" height="${f.size + 40}" rx="${(f.size + 40) / 2}" fill="none" stroke="${p.hi}" stroke-width="4"/>` +
      text(x + 32, y + f.size + 8, f.text, { size: f.size, weight: 600, fill: p.hi }),
  );
}

function bar(x: number, y: number, w: number, h: number, frac: number, p: SlidePalette, delay: number, minWidth = h): string {
  const fw = Math.max(minWidth, w * Math.max(0, Math.min(1, frac)));
  return (
    `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${h / 2}" fill="${p.track}"/>` +
    anim("a-growx", delay, `<rect x="${x}" y="${y}" width="${fw}" height="${h}" rx="${h / 2}" fill="${p.hi}"/>`)
  );
}

/** 24 hour bars in a box; the peak hour in the highlight colour. */
function hourBars(x: number, y: number, w: number, h: number, hours: number[], peak: number | null, p: SlidePalette, delay: number): string {
  const max = Math.max(...hours, 0);
  const gap = 8;
  const bw = (w - gap * 23) / 24;
  let out = "";
  hours.forEach((v, i) => {
    const bh = max > 0 ? Math.max(6, (v / max) * h) : 6;
    const bx = x + i * (bw + gap);
    const fill = i === peak ? p.hi : p.dim;
    const op = i === peak ? 1 : 0.55;
    out += anim(
      "a-growy",
      delay + i * 18,
      `<rect x="${bx.toFixed(1)}" y="${(y + h - bh).toFixed(1)}" width="${bw.toFixed(1)}" height="${bh.toFixed(1)}" rx="${Math.min(8, bw / 2).toFixed(1)}" fill="${fill}" fill-opacity="${op}"/>`,
    );
  });
  for (const t of [0, 6, 12, 18]) {
    out += text(x + t * (bw + gap), y + h + 56, hourLabel(t), { size: 32, weight: 500, fill: p.dim });
  }
  return out;
}

/** A "label / value" pair, value fitted to the width. */
function fact(y: number, label: string, value: string, p: SlidePalette, delay: number, valueMax = 64, x = M, width = CW): string {
  return anim(
    "a-up",
    delay,
    text(x, y, label.toUpperCase(), { size: 32, weight: 700, fill: p.dim, spacing: 4 }) +
      line(x, y + valueMax + 18, value, width, valueMax, { weight: 700, fill: p.ink }, 36),
  );
}

// ---------- the frame every slide shares ----------

// Entrance animations. Elements wait in their start state until the slide's
// layer in the player gets the `play` class (the player keeps the next slide
// parsed and painted ahead of time, so the swap is only an opacity change).
const ANIM_CSS = `
.a-fade,.a-up{opacity:0}
.a-up{transform:translateY(48px)}
.a-growx{transform:scaleX(0);transform-box:fill-box;transform-origin:0 50%}
.a-growy{transform:scaleY(0);transform-box:fill-box;transform-origin:50% 100%}
.play .a-fade{animation:rf .7s ease-out forwards}
.play .a-up{animation:ru .8s cubic-bezier(.2,.8,.2,1) forwards}
.play .a-growx,.play .a-growy{animation:rg 1s cubic-bezier(.2,.8,.2,1) forwards}
@keyframes rf{to{opacity:1}}
@keyframes ru{to{opacity:1;transform:none}}
@keyframes rg{to{transform:none}}
@media (prefers-reduced-motion: reduce){.a-fade,.a-up,.a-growx,.a-growy{animation:none!important;opacity:1;transform:none}}
`;

function frame(kind: string, p: SlidePalette, ctx: SlideCtx, body: string, animate: boolean, landscape: boolean): string {
  const w = landscape ? LANDSCAPE_W : W;
  const h = landscape ? LANDSCAPE_H : H;
  const headerY = landscape ? 112 : 200;
  const size = animate ? `width="100%" height="100%"` : `width="${w}" height="${h}"`;
  const id = `g-${kind}`;
  const label = ctx.recap.label + (ctx.recap.in_progress ? " so far" : "");
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}" ${size} font-family='${FONT}'>` +
    (animate ? `<style>${ANIM_CSS}</style>` : "") +
    `<defs>` +
    `<linearGradient id="${id}-bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${p.bg1}"/><stop offset="1" stop-color="${p.bg2}"/></linearGradient>` +
    `<radialGradient id="${id}-glow"><stop offset="0" stop-color="${p.glow}" stop-opacity="${GLOW_ALPHA}"/><stop offset="1" stop-color="${p.glow}" stop-opacity="0"/></radialGradient>` +
    `</defs>` +
    `<rect width="${w}" height="${h}" fill="url(#${id}-bg)"/>` +
    (landscape
      ? `<circle cx="${w - M}" cy="${h}" r="680" fill="url(#${id}-glow)"/>`
      : decoration(kind, `url(#${id}-glow)`, p)) +
    text(M, headerY, "WRAPPED", { size: 34, weight: 800, fill: p.hi, spacing: 8 }) +
    line(w - M, headerY, label, 560, 34, { weight: 600, fill: p.dim, anchor: "end" }, 24) +
    body +
    `</svg>`
  );
}

/** Each kind's own background shapes (decoration only). */
function decoration(kind: string, glow: string, p: SlidePalette): string {
  const ring = (cx: number, cy: number, r: number, w: number, op: number) =>
    `<circle cx="${cx}" cy="${cy}" r="${r}" fill="none" stroke="${p.hi}" stroke-opacity="${op}" stroke-width="${w}"/>`;
  switch (kind) {
    case "intro":
      return `<circle cx="900" cy="1500" r="700" fill="${glow}"/>` + ring(880, 1480, 320, 6, 0.25) + ring(880, 1480, 460, 3, 0.15) + ring(880, 1480, 600, 2, 0.1);
    case "screen_time":
      return `<circle cx="1000" cy="300" r="620" fill="${glow}"/>` + ring(1000, 300, 380, 40, 0.08);
    case "top_apps":
      return `<circle cx="100" cy="1800" r="640" fill="${glow}"/>`;
    case "time_of_day":
      return `<circle cx="540" cy="2100" r="900" fill="${glow}"/>` + ring(540, 2100, 700, 4, 0.18);
    case "busiest_day":
      return `<circle cx="1080" cy="900" r="700" fill="${glow}"/>` + `<rect x="620" y="1220" width="380" height="380" rx="48" fill="none" stroke="${p.hi}" stroke-opacity="0.18" stroke-width="6"/>`;
    case "records":
      return `<circle cx="0" cy="600" r="600" fill="${glow}"/>`;
    case "git":
      return `<circle cx="980" cy="1700" r="660" fill="${glow}"/>` + ring(900, 1840, 26, 10, 0.3) + `<path d="M900 1866 V1920 M900 1770 V1814" stroke="${p.hi}" stroke-opacity="0.3" stroke-width="10"/>`;
    case "photos":
      return `<circle cx="160" cy="1650" r="640" fill="${glow}"/>` + ring(860, 1560, 150, 12, 0.18) + ring(860, 1560, 90, 6, 0.18);
    case "ai":
      return `<circle cx="540" cy="960" r="820" fill="${glow}"/>`;
    case "disk":
      return `<circle cx="900" cy="1700" r="700" fill="${glow}"/>` + `<rect x="700" y="1420" width="280" height="360" rx="28" fill="none" stroke="${p.hi}" stroke-opacity="0.18" stroke-width="8"/>`;
    default:
      return `<circle cx="540" cy="1920" r="900" fill="${glow}"/>`;
  }
}

// ---------- slides ----------

const KIND_WORD: Record<string, string> = { week: "week", month: "month", year: "year" };

const appName = (ctx: SlideCtx, id: string) => ctx.names[id] ?? id;

function appTime(secs: number, share: number): string {
  const pct = share * 100;
  return `${hm(secs)} · ${pct > 0 && pct < 1 ? "<1" : Math.round(pct)}%`;
}

function timeOfDay(s: Extract<SlideJson, { kind: "time_of_day" }>) {
  // Match the local hour buckets in core's recap::day_label.
  const [title, share, range] =
    s.label === "night_owl" ? ["Night owl", s.night_share, "22:00–05:00"]
    : s.label === "early_bird" ? ["Early bird", s.morning_share, "05:00–12:00"]
    : ["Daytime", 1 - s.night_share - s.morning_share, "12:00–22:00"];
  return {
    title,
    detail: `${Math.round(share * 100)}% of screen time · ${range}`,
    peak: `${hourLabel(s.peak_hour)}–${hourLabel((s.peak_hour + 1) % 24)}`,
  };
}

interface RecordValue {
  label: string;
  value: string;
  detail: string;
  count?: { n: number; f: Fmt };
}

function recordValues(s: Extract<SlideJson, { kind: "records" }>, ctx: SlideCtx): RecordValue[] {
  const entries: RecordValue[] = [];
  if (s.longest_session) {
    const { secs, app } = s.longest_session;
    entries.push({
      label: "Longest session",
      value: hm(secs),
      detail: `in ${appName(ctx, app)}`,
      count: { n: secs, f: "hm" },
    });
  }
  if (s.latest_night) {
    const { night, clock_min } = s.latest_night;
    const { y, m, d } = parseDay(night);
    const weekday = WEEKDAYS[(new Date(Date.UTC(y, m - 1, d)).getUTCDay() + 6) % 7]?.slice(0, 3);
    entries.push({ label: "Latest night", value: clock(clock_min), detail: `the night of ${weekday} ${shortDate(night)}` });
  }
  if (s.streak) {
    const { days, from, to } = s.streak;
    entries.push({
      label: "Longest streak",
      value: fmt("days", days),
      detail: days === 1 ? shortDate(from) : `${shortDate(from)} – ${shortDate(to)}`,
      count: { n: days, f: "days" },
    });
  }
  return entries;
}

function recordBlock(record: RecordValue, x: number, y: number, width: number, max: number, delay: number, p: SlidePalette): string {
  const fitted = fit(record.value, 800, max, width, 60);
  return anim(
    "a-up",
    delay,
    text(x, y, record.label.toUpperCase(), { size: 34, weight: 700, fill: p.dim, spacing: 4 }) +
      text(x, y + max + 10, fitted.text, {
        size: fitted.size,
        weight: 800,
        fill: p.hi,
        ...(fitted.text === record.value ? record.count : {}),
      }) +
      line(x, y + max + 90, record.detail, width, 48, { weight: 600, fill: p.ink }, 28),
  );
}

const SUMMARY_LABELS: Record<string, string> = {
  screen_time: "Screen time",
  top_app: "Top app",
  commits: "Commits",
  photos: "Photos",
  ai_tokens: "AI tokens",
  streak: "Longest streak",
  disk_written: "Disk written",
  active_days: "Active days",
};

function summaryValue(it: Extract<SlideJson, { kind: "summary" }>["items"][number], ctx: SlideCtx) {
  const label = SUMMARY_LABELS[it.id] ?? it.id;
  if (it.id === "top_app") return { label, value: appName(ctx, it.text ?? "") };
  const formats: Record<string, Fmt> = { screen_time: "hm", ai_tokens: "tokens", disk_written: "gb", streak: "days" };
  const f = formats[it.id] ?? "int";
  return { label, value: fmt(f, it.value), f };
}

function body(s: SlideJson, ctx: SlideCtx, p: SlidePalette): string {
  const r = ctx.recap;
  switch (s.kind) {
    case "intro": {
      const title =
        r.kind === "year" ? `Your ${r.range.from.slice(0, 4)}` : `Your ${KIND_WORD[r.kind] ?? r.kind}`;
      let out =
        line(M, 640, title, CW, 150, { weight: 800, fill: p.ink, a: "a-up", d: 0 }) +
        line(M, 790, "wrapped.", CW, 150, { weight: 800, fill: p.hi, a: "a-up", d: 120 }) +
        line(M, 900, r.label + (r.in_progress ? " · so far" : ""), CW, 52, { weight: 600, fill: p.dim, a: "a-fade", d: 300 });
      if (s.screen_secs) {
        const n = bigNumber(M, 1250, s.screen_secs, "hm", CW, 220, { fill: p.hi, a: "a-up", d: 450 });
        out += n.svg + text(M, 1340, "on screen", { size: 56, weight: 600, fill: p.ink, a: "a-fade", d: 600 });
      }
      out += chip(M, 1460, `${s.active_days} active ${s.active_days === 1 ? "day" : "days"} of ${s.days}`, p, 750);
      return out;
    }
    case "screen_time": {
      let out = kicker("Screen time", p);
      out += bigNumber(M, 640, s.total_secs, "hm", CW, 240, { fill: p.hi, a: "a-up", d: 100 }).svg;
      out += line(M, 790, `That's ${hm(s.daily_avg_secs)} a day`, CW, 64, { weight: 700, fill: p.ink, a: "a-fade", d: 350 });
      out += text(M, 860, `on ${s.screen_days} ${s.screen_days === 1 ? "day" : "days"} with screen time`, {
        size: 40,
        weight: 500,
        fill: p.dim,
        a: "a-fade",
        d: 450,
      });
      out += chip(M, 940, changeText(s.change_pct, ctx), p, 600);
      if (s.busiest_weekday && r.kind !== "week") {
        out += fact(1300, "Busiest weekday", `${WEEKDAYS[s.busiest_weekday.weekday]}s`, p, 800, 96);
        out += text(M, 1500, `${hm(s.busiest_weekday.avg_secs)} on average`, { size: 44, weight: 600, fill: p.dim, a: "a-fade", d: 950 });
      }
      return out;
    }
    case "top_apps": {
      let out = kicker("Top apps", p);
      const rowH = 250;
      s.apps.slice(0, 5).forEach((a, i) => {
        const y = 520 + i * rowH;
        const name = appName(ctx, a.key);
        const rowX = M + 110;
        const rowWidth = CW - 110;
        const nameMax = i === 0 ? 76 : 60;
        out += anim(
          "a-up",
          120 + i * 110,
          text(M, y, String(i + 1), { size: i === 0 ? 96 : 72, weight: 800, fill: p.hi }) +
            line(rowX, y, name, rowWidth, nameMax, { weight: 700, fill: p.ink }, Math.round(nameMax * 0.75)) +
            text(rowX, y + 60, appTime(a.secs, a.share), { size: 40, weight: 600, fill: p.dim }),
        );
        out += bar(rowX, y + 84, rowWidth, 22, a.share, p, 250 + i * 110, 0);
      });
      return out;
    }
    case "time_of_day": {
      const { title, detail, peak } = timeOfDay(s);
      let out = kicker("Time of day", p);
      out += line(M, 630, title, CW, 190, { weight: 800, fill: p.hi, a: "a-up", d: 200 });
      out += line(M, 750, detail, CW, 46, { weight: 600, fill: p.dim, a: "a-fade", d: 400 }, 36);
      out += hourBars(M, 960, CW, 520, s.hours, s.peak_hour, p, 500);
      out += anim(
        "a-up",
        900,
        text(M, 1680, "PEAK HOUR · LOCAL TIME", { size: 32, weight: 700, fill: p.dim, spacing: 4 }) +
          text(M, 1780, peak, { size: 96, weight: 800, fill: p.ink }),
      );
      return out;
    }
    case "busiest_day": {
      let out = kicker("Busiest day", p);
      out += line(M, 720, WEEKDAYS[s.weekday] ?? "", CW, 210, { weight: 800, fill: p.hi, a: "a-up", d: 100 });
      out += line(M, 830, longDate(s.date), CW, 72, { weight: 700, fill: p.ink, a: "a-fade", d: 300 });
      out += bigNumber(M, 1150, s.secs, "hm", 560, 150, { fill: p.ink, a: "a-up", d: 500 }).svg;
      out += text(M, 1240, "on screen that day", { size: 48, weight: 600, fill: p.dim, a: "a-fade", d: 650 });
      return out;
    }
    case "records": {
      let out = kicker("Records", p);
      const records = recordValues(s, ctx);
      const start = records.length === 1 ? 920 : records.length === 2 ? 690 : 520;
      const gap = records.length === 2 ? 460 : 400;
      records.forEach((record, i) => {
        out += recordBlock(record, M, start + i * gap, CW, 150, 100 + i * 200, p);
      });
      return out;
    }
    case "git": {
      let out = kicker("Git", p);
      out += bigNumber(M, 700, s.commits, "int", CW, 300, { fill: p.hi, a: "a-up", d: 100 }).svg;
      out += text(M, 800, s.commits === 1 ? "commit" : "commits", { size: 72, weight: 700, fill: p.ink, a: "a-fade", d: 300 });
      out += chip(M, 870, changeText(s.change_pct, ctx), p, 450);
      if (s.top_repo) out += fact(1110, "Top repository", s.top_repo.key, p, 600, 72);
      if (s.busiest_hour !== null) {
        out += anim(
          "a-fade",
          700,
          text(M, 1350, `BUSIEST HOUR · ${hourLabel(s.busiest_hour)}`, { size: 32, weight: 700, fill: p.dim, spacing: 4 }),
        );
        out += hourBars(M, 1400, CW, 260, s.hours, s.busiest_hour, p, 750);
      }
      return out;
    }
    case "photos": {
      let out = kicker("Photos", p);
      out += bigNumber(M, 700, s.count, "int", CW, 300, { fill: p.hi, a: "a-up", d: 100 }).svg;
      out += text(M, 800, s.count === 1 ? "photo taken" : "photos taken", { size: 72, weight: 700, fill: p.ink, a: "a-fade", d: 300 });
      out += chip(M, 870, changeText(s.change_pct, ctx), p, 450);
      let y = 1110;
      let d = 600;
      if (s.top_camera) {
        out += fact(y, "Camera", s.top_camera.key, p, d);
        y += 190;
        d += 120;
      }
      if (s.top_lens) {
        out += fact(y, "Lens", s.top_lens.key, p, d);
        y += 190;
        d += 120;
      }
      if (s.top_focal) out += fact(y, "Favourite focal length", `${s.top_focal.key} mm`, p, d);
      return out;
    }
    case "ai": {
      let out = kicker("AI usage", p);
      out += bigNumber(M, 660, s.tokens, "tokens", CW, 260, { fill: p.hi, a: "a-up", d: 100 }).svg;
      out += text(M, 750, "tokens in + out", { size: 60, weight: 700, fill: p.ink, a: "a-fade", d: 300 });
      const total = s.input + s.output || 1;
      out += bar(M, 820, CW, 26, s.input / total, p, 400);
      out += text(M, 900, `${tokens(s.input)} in · ${tokens(s.output)} out`, { size: 40, weight: 600, fill: p.dim, a: "a-fade", d: 500 });
      out += chip(M, 960, changeText(s.change_pct, ctx), p, 600);
      let y = 1220;
      let d = 700;
      for (const [label, k, name] of [
        ["Top tool", s.top_tool, (x: string) => aiToolName(x)],
        ["Top model", s.top_model, modelLabel],
        ["Top project", s.top_project, (x: string) => x],
      ] as const) {
        if (!k) continue;
        out += fact(y, label, name(k.key), p, d, 60);
        y += 180;
        d += 120;
      }
      return out;
    }
    case "disk": {
      let out = kicker("Disk", p);
      out += bigNumber(M, 760, s.bytes_written, "gb", CW, 260, { fill: p.hi, a: "a-up", d: 100 }).svg;
      out += text(M, 860, "written to your internal disk", { size: 52, weight: 700, fill: p.ink, a: "a-fade", d: 300 });
      out += chip(M, 940, changeText(s.change_pct, ctx), p, 450);
      return out;
    }
    case "summary": {
      let out = line(M, 480, `Your ${r.kind === "year" ? r.range.from.slice(0, 4) : KIND_WORD[r.kind]}`, CW, 110, {
        weight: 800,
        fill: p.ink,
        a: "a-up",
        d: 0,
      });
      out += line(M, 590, "in numbers", CW, 110, { weight: 800, fill: p.hi, a: "a-up", d: 100 });
      const cellH = 220;
      s.items.forEach((it, i) => {
        const y = 700 + i * cellH;
        const { label, value, f } = summaryValue(it, ctx);
        const fitted = fit(value, 800, 110, CW, f ? 44 : 64);
        out += `<rect x="${M}" y="${y + 12}" width="${CW}" height="3" fill="${p.dim}" fill-opacity="0.35"/>`;
        out += anim(
          "a-up",
          200 + i * 120,
          text(M, y + 72, label.toUpperCase(), { size: 32, weight: 700, fill: p.dim, spacing: 4 }) +
            text(M, y + 72 + 12 + fitted.size * 0.9, fitted.text, {
              size: fitted.size,
              weight: 800,
              fill: i === 0 ? p.hi : p.ink,
              ...(f && fitted.text === value ? { n: it.value, f } : {}),
            }),
        );
      });
      return out;
    }
  }
}

function landscapeBody(s: SlideJson, ctx: SlideCtx, p: SlidePalette): string {
  const r = ctx.recap;
  const width = LANDSCAPE_W - 2 * M;
  const right = 850;
  const leftWidth = right - M - 64;
  const rightWidth = LANDSCAPE_W - M - right;
  const period = r.kind === "year" ? r.range.from.slice(0, 4) : KIND_WORD[r.kind];
  const heading = (label: string) => kicker(label, p, 230);
  const hero = (value: number, f: Fmt, max = 146) =>
    bigNumber(M, 490, value, f, leftWidth, max, { fill: p.hi, a: "a-up", d: 100 }).svg;
  const caption = (label: string, y = 568) =>
    line(M, y, label, leftWidth, 46, { weight: 600, fill: p.ink, a: "a-fade", d: 350 }, 32);
  const change = (pct: number | null, y = 628) => chip(M, y, changeText(pct, ctx), p, 500, leftWidth);

  switch (s.kind) {
    case "intro": {
      const active = `${s.active_days} active ${s.active_days === 1 ? "day" : "days"} of ${s.days}`;
      let out =
        line(M, 440, `Your ${period ?? r.kind}`, leftWidth, 128, { weight: 800, fill: p.ink, a: "a-up", d: 0 }) +
        line(M, 580, "wrapped.", leftWidth, 128, { weight: 800, fill: p.hi, a: "a-up", d: 120 });
      if (s.screen_secs) {
        out += bigNumber(right, 476, s.screen_secs, "hm", rightWidth, 126, { fill: p.hi, a: "a-up", d: 350 }).svg;
        out += text(right, 550, "on screen", { size: 44, weight: 600, fill: p.ink, a: "a-fade", d: 500 });
      }
      return out + chip(right, s.screen_secs ? 598 : 430, active, p, 650, rightWidth);
    }
    case "screen_time": {
      let out = heading("Screen time") + hero(s.total_secs, "hm") + caption(`That's ${hm(s.daily_avg_secs)} a day`);
      out += text(M, 626, `on ${s.screen_days} ${s.screen_days === 1 ? "day" : "days"} with screen time`, {
        size: 34, weight: 500, fill: p.dim, a: "a-fade", d: 450,
      });
      out += change(s.change_pct, 672);
      if (s.busiest_weekday && r.kind !== "week") {
        out += fact(393, "Busiest weekday", `${WEEKDAYS[s.busiest_weekday.weekday]}s`, p, 650, 86, right, rightWidth);
        out += text(right, 563, `${hm(s.busiest_weekday.avg_secs)} on average`, {
          size: 40, weight: 600, fill: p.dim, a: "a-fade", d: 800,
        });
      }
      return out;
    }
    case "top_apps": {
      const apps = s.apps.slice(0, 5);
      const barX = 815;
      const barWidth = 375;
      const firstY = 348 + (5 - apps.length) * 50;
      let out = heading("Top apps");
      apps.forEach((a, i) => {
        const y = firstY + i * 100;
        const delay = 120 + i * 110;
        out += anim(
          "a-up",
          delay,
          text(M, y, String(i + 1), { size: 52, weight: 800, fill: p.hi }) +
            line(M + 74, y, appName(ctx, a.key), barX - M - 114, 54, { weight: 700, fill: p.ink }, 40) +
            line(LANDSCAPE_W - M, y, appTime(a.secs, a.share), 290, 38, {
              weight: 600, fill: p.dim, anchor: "end",
            }, 32),
        );
        out += bar(barX, y - 30, barWidth, 24, a.share, p, delay + 130, 0);
      });
      return out;
    }
    case "time_of_day": {
      const { title, detail, peak } = timeOfDay(s);
      return heading("Time of day") +
        line(M, 435, title, leftWidth, 122, { weight: 800, fill: p.hi, a: "a-up", d: 150 }) +
        line(M, 507, detail, leftWidth, 40, { weight: 600, fill: p.dim, a: "a-fade", d: 350 }, 32) +
        fact(640, "Peak hour · local time", peak, p, 700, 62, M, leftWidth) +
        hourBars(right, 330, rightWidth, 350, s.hours, s.peak_hour, p, 400);
    }
    case "busiest_day":
      return heading("Busiest day") +
        line(M, 475, WEEKDAYS[s.weekday] ?? "", leftWidth, 140, { weight: 800, fill: p.hi, a: "a-up", d: 100 }) +
        line(M, 565, longDate(s.date), leftWidth, 52, { weight: 700, fill: p.ink, a: "a-fade", d: 300 }) +
        bigNumber(right, 475, s.secs, "hm", rightWidth, 126, { fill: p.ink, a: "a-up", d: 500 }).svg +
        text(right, 565, "on screen that day", { size: 44, weight: 600, fill: p.dim, a: "a-fade", d: 650 });
    case "records": {
      const records = recordValues(s, ctx);
      const gap = 48;
      const start = records.length === 1 ? M + 220 : M;
      const columnWidth = (LANDSCAPE_W - 2 * start - gap * (records.length - 1)) / Math.max(1, records.length);
      return heading("Records") + records.map((record, i) =>
        recordBlock(record, start + i * (columnWidth + gap), 425, columnWidth, records.length === 3 ? 116 : 140, 100 + i * 180, p),
      ).join("");
    }
    case "git": {
      let out = heading("Git") + hero(s.commits, "int") + caption(s.commits === 1 ? "commit" : "commits") + change(s.change_pct);
      if (s.top_repo) out += fact(330, "Top repository", s.top_repo.key, p, 550, 62, right, rightWidth);
      if (s.busiest_hour !== null) {
        out += text(right, 515, `BUSIEST HOUR · ${hourLabel(s.busiest_hour)}`, {
          size: 32, weight: 700, fill: p.dim, spacing: 3, a: "a-fade", d: 650,
        });
        out += hourBars(right, 545, rightWidth, 190, s.hours, s.busiest_hour, p, 750);
      }
      return out;
    }
    case "photos": {
      let out = heading("Photos") + hero(s.count, "int") + caption(s.count === 1 ? "photo taken" : "photos taken") + change(s.change_pct);
      let y = 340;
      for (const [label, entry, suffix] of [
        ["Favourite camera", s.top_camera, ""],
        ["Favourite lens", s.top_lens, ""],
        ["Favourite focal length", s.top_focal, " mm"],
      ] as const) {
        if (!entry) continue;
        out += fact(y, label, entry.key + suffix, p, 550 + y - 340, 54, right, rightWidth);
        y += 158;
      }
      return out;
    }
    case "ai": {
      const inputShare = s.input / (s.input + s.output || 1);
      let out = heading("AI usage") + hero(s.tokens, "tokens", 138) + caption("tokens in + out");
      out += bar(M, 603, leftWidth, 22, inputShare, p, 450);
      out += caption(`${tokens(s.input)} in · ${tokens(s.output)} out`, 682);
      out += change(s.change_pct, 729);
      let y = 340;
      for (const [label, entry, name] of [
        ["Top tool", s.top_tool, aiToolName],
        ["Top model", s.top_model, modelLabel],
        ["Top project", s.top_project, (x: string) => x],
      ] as const) {
        if (!entry) continue;
        out += fact(y, label, name(entry.key), p, 550 + y - 340, 54, right, rightWidth);
        y += 158;
      }
      return out;
    }
    case "disk":
      return heading("Disk") +
        bigNumber(M, 525, s.bytes_written, "gb", width, 192, { fill: p.hi, a: "a-up", d: 100 }).svg +
        caption("written to your internal disk", 618) + change(s.change_pct, 680);
    case "summary": {
      let out = line(M, 285, `Your ${period} in numbers`, width, 100, { weight: 800, fill: p.ink, a: "a-up", d: 0 });
      const columns = Math.min(3, Math.max(1, s.items.length));
      const gap = 48;
      const cellWidth = (width - gap * (columns - 1)) / columns;
      s.items.forEach((item, i) => {
        const x = M + (i % columns) * (cellWidth + gap);
        const y = 390 + Math.floor(i / columns) * 155;
        const { label, value, f } = summaryValue(item, ctx);
        const fitted = fit(value, 800, 76, cellWidth, 42);
        out += anim(
          "a-up",
          200 + i * 110,
          `<rect x="${x}" y="${y}" width="${cellWidth}" height="3" fill="${p.dim}" fill-opacity="0.35"/>` +
            text(x, y + 53, label.toUpperCase(), { size: 32, weight: 700, fill: p.dim, spacing: 4 }) +
            text(x, y + 135, fitted.text, {
              size: fitted.size, weight: 800, fill: i === 0 ? p.hi : p.ink,
              ...(f && fitted.text === value ? { n: item.value, f } : {}),
            }),
        );
      });
      return out;
    }
  }
}

/** One slide as a complete SVG document. `animate`: with the entrance
 *  animations and sized to its container (screen); otherwise static at
 *  1080 × 1920 (export). */
export function slideSvg(s: SlideJson, ctx: SlideCtx, animate: boolean, orientation: "landscape" | "portrait" = "portrait"): string {
  const p = slidePalette(ctx.accent, ctx.theme, s.kind);
  const landscape = animate && orientation === "landscape";
  return frame(s.kind, p, ctx, landscape ? landscapeBody(s, ctx, p) : body(s, ctx, p), animate, landscape);
}

/** File name of a slide in an export: "03-top_apps.png". */
export const slideFile = (i: number, s: SlideJson) => `${String(i + 1).padStart(2, "0")}-${s.kind}.png`;

/** The current theme and accent, read from <html>. */
export function currentLook(): { theme: "dark" | "light"; accent: string } {
  const el = document.documentElement;
  const theme = el.dataset.theme === "light" ? "light" : "dark";
  const accent = getComputedStyle(el).getPropertyValue("--accent").trim() || "#3ddc97";
  return { theme, accent };
}

/** Render a static slide SVG to PNG bytes (1080 × 1920). */
export async function svgToPng(svg: string): Promise<Uint8Array> {
  const img = new Image();
  img.decoding = "async";
  img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg);
  await img.decode();
  const canvas = document.createElement("canvas");
  canvas.width = W;
  canvas.height = H;
  const c = canvas.getContext("2d");
  if (!c) throw new Error("no canvas");
  c.drawImage(img, 0, 0, W, H);
  const blob = await new Promise<Blob | null>((res) => canvas.toBlob(res, "image/png"));
  if (!blob) throw new Error("PNG encoding failed");
  return new Uint8Array(await blob.arrayBuffer());
}
