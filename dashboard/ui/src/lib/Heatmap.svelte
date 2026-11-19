<script lang="ts">
  // GitHub-style year grid: one column per week, rows Monday..Sunday.
  // Click a day to select it, drag for a range, shift-click to extend.
  // Arrow keys move the selection (shift extends). Hand-drawn SVG.
  import type { HeatmapMetric, YearSeries } from "./types";
  import {
    type Range,
    type Ymd,
    addDays,
    dayNumber,
    fmtDay,
    fromDayNumber,
    monthShort,
    ordered,
    weekday,
    weekdayShort,
  } from "./dates";
  import Select from "./Select.svelte";
  import { fmtTokens, fmtValue } from "./format";

  interface Props {
    year: number;
    minYear: number;
    maxYear: number;
    today: Ymd;
    metrics: HeatmapMetric[];
    metric: string;
    series: YearSeries | null;
    loading: boolean;
    error: string | null;
    selection: Range;
    /** `final` is false while dragging (no fetch until the drag ends). */
    onselect: (r: Range, final: boolean) => void;
    onyear: (y: number) => void;
    onmetric: (id: string) => void;
  }
  let {
    year,
    minYear,
    maxYear,
    today,
    metrics,
    metric,
    series,
    loading,
    error,
    selection,
    onselect,
    onyear,
    onmetric,
  }: Props = $props();

  type Pt = [number, number];

  /** A closed polygon of axis-aligned points with rounded corners. */
  function roundedPath(pts: Pt[], r: number): string {
    // Drop repeated and collinear points so every corner is a real one.
    const p = pts.filter((q, i) => {
      const a = pts[(i + pts.length - 1) % pts.length]!;
      const b = pts[(i + 1) % pts.length]!;
      if (q[0] === a[0] && q[1] === a[1]) return false;
      return !((a[0] === q[0] && q[0] === b[0]) || (a[1] === q[1] && q[1] === b[1]));
    });
    const n = p.length;
    const toward = (from: Pt, to: Pt, d: number): Pt => {
      const len = Math.hypot(to[0] - from[0], to[1] - from[1]);
      return [from[0] + ((to[0] - from[0]) / len) * d, from[1] + ((to[1] - from[1]) / len) * d];
    };
    let d = "";
    for (let i = 0; i < n; i++) {
      const prev = p[(i + n - 1) % n]!;
      const cur = p[i]!;
      const next = p[(i + 1) % n]!;
      const rr = Math.min(
        r,
        Math.hypot(prev[0] - cur[0], prev[1] - cur[1]) / 2,
        Math.hypot(next[0] - cur[0], next[1] - cur[1]) / 2,
      );
      const a = toward(cur, prev, rr);
      const b = toward(cur, next, rr);
      d += `${i === 0 ? "M" : "L"}${a[0]} ${a[1]}Q${cur[0]} ${cur[1]} ${b[0]} ${b[1]}`;
    }
    return `${d}Z`;
  }

  const CELL = 12;
  const STEP = 15;
  const LEFT = 30;
  const TOP = 18;

  const jan1 = $derived(dayNumber(`${year}-01-01`));
  const nDays = $derived(dayNumber(`${year}-12-31`) - jan1 + 1);
  const offset = $derived(weekday(`${year}-01-01`));
  const weeks = $derived(Math.ceil((nDays + offset) / 7));
  const width = $derived(LEFT + weeks * STEP);
  const height = TOP + 7 * STEP;

  // Values for this grid only when the series is for this year.
  const values = $derived(
    series && series.range.from === `${year}-01-01` ? series.days : null,
  );

  // Four levels by rank among the days with a value (quartiles), so one
  // outlier does not wash out the rest.
  const sorted = $derived(values ? values.filter((v) => v > 0).sort((a, b) => a - b) : []);
  function level(v: number): number {
    if (!(v > 0) || sorted.length === 0) return 0;
    let lo = 0;
    let hi = sorted.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if ((sorted[mid] ?? 0) <= v) lo = mid + 1;
      else hi = mid;
    }
    return Math.max(1, Math.ceil((lo / sorted.length) * 4));
  }

  interface Cell {
    i: number;
    date: Ymd;
    x: number;
    y: number;
    value: number;
    level: number;
    future: boolean;
  }
  const cells: Cell[] = $derived.by(() => {
    const out: Cell[] = [];
    const todayN = dayNumber(today);
    for (let i = 0; i < nDays; i++) {
      const k = i + offset;
      const value = values?.[i] ?? 0;
      out.push({
        i,
        date: fromDayNumber(jan1 + i),
        x: LEFT + Math.floor(k / 7) * STEP,
        y: TOP + (k % 7) * STEP,
        value,
        level: level(value),
        future: jan1 + i > todayN,
      });
    }
    return out;
  });

  const monthLabels = $derived(
    Array.from({ length: 12 }, (_, m) => {
      const k = dayNumber(`${year}-${String(m + 1).padStart(2, "0")}-01`) - jan1 + offset;
      return { m, x: LEFT + Math.floor(k / 7) * STEP };
    }),
  );

  const isAi = $derived(metric.startsWith("ai_usage/"));
  const fmt = (v: number): string => (isAi ? fmtTokens(v) : fmtValue(v, unit));
  const unit = $derived(series?.unit ?? metrics.find((m) => m.id === metric)?.unit ?? "count");
  const daysWithData = $derived(sorted.length);
  const yearTotal = $derived(values ? values.reduce((a, b) => a + b, 0) : 0);

  const inSelection = (d: Ymd): boolean => selection.from <= d && d <= selection.to;
  const single = $derived(selection.from === selection.to);

  /** One thin rounded outline around the outer edge of the selected days
   *  (clipped to this year), as a single SVG path. Not for a single day (it
   *  has its own ring) or a selection outside the year. */
  const outline = $derived.by((): string | null => {
    if (single) return null;
    const a = Math.max(dayNumber(selection.from) - jan1, 0);
    // Never around future days.
    const b = Math.min(dayNumber(selection.to) - jan1, nDays - 1, dayNumber(today) - jan1);
    if (a > b) return null;
    const pad = (STEP - CELL) / 2;
    const x0 = (c: number) => LEFT + c * STEP - pad;
    const x1 = (c: number) => LEFT + c * STEP + CELL + pad;
    const yt = (r: number) => TOP + r * STEP - pad;
    const yb = (r: number) => TOP + r * STEP + CELL + pad;
    const [c1, r1] = [Math.floor((a + offset) / 7), (a + offset) % 7];
    const [c2, r2] = [Math.floor((b + offset) / 7), (b + offset) % 7];
    const box = (ca: number, cb: number, ra: number, rb: number): Pt[] => [
      [x0(ca), yt(ra)],
      [x1(cb), yt(ra)],
      [x1(cb), yb(rb)],
      [x0(ca), yb(rb)],
    ];
    if (c1 === c2) return roundedPath(box(c1, c2, r1, r2), 4);
    // Two pieces that only meet at a corner (or not at all): one outline each.
    if (c2 === c1 + 1 && r2 < r1) {
      return roundedPath(box(c1, c1, r1, 6), 4) + roundedPath(box(c2, c2, 0, r2), 4);
    }
    return roundedPath(
      [
        [x0(c1), yt(r1)],
        [x0(c1 + 1), yt(r1)],
        [x0(c1 + 1), yt(0)],
        [x1(c2), yt(0)],
        [x1(c2), yb(r2)],
        [x0(c2), yb(r2)],
        [x0(c2), yb(6)],
        [x0(c1), yb(6)],
      ],
      4,
    );
  });

  // ---------- pointer and keyboard ----------

  let svg: SVGSVGElement | undefined = $state();
  let wrap: HTMLDivElement | undefined = $state();
  let anchor: Ymd | null = null;
  let cursor: Ymd | null = null;
  let dragging = false;
  let live: Range | null = null;
  let hover: { cell: Cell; x: number; y: number } | null = $state(null);

  /** The day under the pointer; while dragging, clamped into the grid. */
  function dayAt(e: PointerEvent, clamp: boolean): Cell | null {
    if (!svg) return null;
    const ctm = svg.getScreenCTM();
    if (!ctm) return null;
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(ctm.inverse());
    let col = Math.floor((p.x - LEFT) / STEP);
    let row = Math.floor((p.y - TOP) / STEP);
    if (clamp) {
      col = Math.min(Math.max(col, 0), weeks - 1);
      row = Math.min(Math.max(row, 0), 6);
    } else if (col < 0 || col >= weeks || row < 0 || row > 6) {
      return null;
    }
    let i = col * 7 + row - offset;
    if (clamp) i = Math.min(Math.max(i, 0), nDays - 1);
    return cells[i] ?? null;
  }

  function emit(r: Range, final: boolean) {
    live = r;
    onselect(r, final);
  }

  // A selection made elsewhere (presets, dates) becomes the new anchor.
  $effect(() => {
    const s = selection;
    if (!live || live.from !== s.from || live.to !== s.to) {
      anchor = s.from;
      cursor = s.to;
      live = null;
    }
  });

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    const c = dayAt(e, false);
    if (!c) return;
    svg?.setPointerCapture(e.pointerId);
    svg?.focus({ preventScroll: true });
    dragging = true;
    if (e.shiftKey) {
      anchor ??= selection.from;
    } else {
      anchor = c.date;
    }
    cursor = c.date;
    emit(ordered(anchor, c.date), false);
  }

  function onpointermove(e: PointerEvent) {
    const c = dayAt(e, dragging);
    if (wrap && c && !dragging) {
      const box = wrap.getBoundingClientRect();
      hover = { cell: c, x: e.clientX - box.left, y: e.clientY - box.top };
    } else {
      hover = null;
    }
    if (dragging && c && anchor) {
      const r = ordered(anchor, c.date);
      cursor = c.date;
      if (!live || live.from !== r.from || live.to !== r.to) emit(r, false);
    }
  }

  function onpointerup() {
    if (!dragging) return;
    dragging = false;
    if (live) emit(live, true);
  }

  function onkeydown(e: KeyboardEvent) {
    const step = { ArrowUp: -1, ArrowDown: 1, ArrowLeft: -7, ArrowRight: 7 }[e.key];
    if (step === undefined) return;
    e.preventDefault();
    const from = cursor ?? selection.to;
    const first = `${year}-01-01`;
    const last = `${year}-12-31`;
    let next = addDays(from, step);
    if (next < first) next = first;
    if (next > last) next = last;
    cursor = next;
    if (e.shiftKey) {
      anchor ??= selection.from;
      emit(ordered(anchor, next), true);
    } else {
      anchor = next;
      emit({ from: next, to: next }, true);
    }
  }

  const fill = (c: Cell): string =>
    c.future
      ? "var(--cell-future)"
      : c.level === 0
        ? inSelection(c.date) && !single
          ? "var(--heat-empty-selected)"
          : "var(--cell-empty)"
        : `var(--cell-${c.level})`;
</script>

<section class="card heatmap">
  <header>
    <div class="title">
      <h2>Activity</h2>
      <div class="year-nav">
        <button aria-label="Previous year" disabled={year <= minYear} onclick={() => onyear(year - 1)}>‹</button>
        <span class="num year">{year}</span>
        <button aria-label="Next year" disabled={year >= maxYear} onclick={() => onyear(year + 1)}>›</button>
      </div>
    </div>
    <div class="controls">
      {#if values}
        <span class="muted summary">
          <span class="num">{daysWithData}</span> days with data{#if series?.agg === "sum" && yearTotal > 0}{" "}· <span class="num">{fmt(yearTotal)}</span> in {year}{/if}
        </span>
      {/if}
      <div class="muted">
        <Select
          value={metric}
          ariaLabel="Colour by"
          onchange={onmetric}
          options={metrics.map((m) => ({ value: m.id, label: m.title }))}
        />
      </div>
    </div>
  </header>

  <div class="grid-wrap" bind:this={wrap} class:loading>
    <svg
      bind:this={svg}
      viewBox="0 0 {width} {height}"
      role="grid"
      aria-label="Activity in {year}: click a day, drag or shift-click for a range, arrow keys to move"
      tabindex="0"
      {onpointerdown}
      {onpointermove}
      {onpointerup}
      onpointercancel={onpointerup}
      onpointerleave={() => (hover = null)}
      {onkeydown}
    >
      {#each monthLabels as l (l.m)}
        <text class="label" x={l.x} y={TOP - 6}>{monthShort(l.m)}</text>
      {/each}
      {#each [0, 2, 4] as r (r)}
        <text class="label" x={0} y={TOP + r * STEP + CELL - 2}>{weekdayShort(r)}</text>
      {/each}
      {#each cells as c (c.i)}
        <rect
          x={c.x}
          y={c.y}
          width={CELL}
          height={CELL}
          rx="2.5"
          fill={fill(c)}
          class:dim={!inSelection(c.date)}
          class:ring={single && !c.future && c.date === selection.from}
          class:today={c.date === today}
        />
      {/each}
      {#if outline}
        <path class="outline" d={outline} />
      {/if}
    </svg>

    {#if hover}
      <div class="tooltip" style="left: {hover.x}px; top: {hover.y}px">
        <div>{fmtDay(hover.cell.date)}</div>
        <div class="num value">
          {hover.cell.future ? "—" : values ? fmt(hover.cell.value) : "…"}
        </div>
      </div>
    {/if}
  </div>

  <footer>
    {#if error}
      <span class="error">{error}</span>
    {:else}
      <span class="faint">Click a day · drag or shift-click for a range</span>
    {/if}
    <span class="legend faint">
      Less
      {#each [0, 1, 2, 3, 4] as l (l)}
        <i style="background: {l === 0 ? 'var(--cell-empty)' : `var(--cell-${l})`}"></i>
      {/each}
      More
    </span>
  </footer>
</section>

<style>
  .heatmap {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  header,
  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .title,
  .controls,
  .year-nav {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .year-nav {
    gap: 6px;
  }
  .year-nav button {
    padding: 0 9px;
    line-height: 22px;
  }
  .year {
    min-width: 3.2em;
    text-align: center;
    font-size: 15px;
  }
  .summary {
    font-size: 12px;
  }
  /* Proportional digits here: the mono space in "22h 27m" looks like two. */
  .summary .num {
    font-family: var(--font);
  }
  .grid-wrap {
    position: relative;
  }
  .grid-wrap.loading svg {
    opacity: 0.55;
    transition: opacity 0.2s;
  }
  svg {
    display: block;
    width: 100%;
    height: auto;
    touch-action: none;
    border-radius: 4px;
  }
  .label {
    fill: var(--text-faint);
    font-size: 9px;
    font-family: var(--font);
  }
  rect {
    shape-rendering: geometricPrecision;
  }
  rect.today {
    stroke: rgba(var(--accent-rgb), 0.9);
    stroke-width: 1;
  }
  /* A selection keeps its cells' colours; everything else steps back. */
  rect {
    transition: opacity 0.12s;
  }
  rect.dim {
    opacity: 0.28;
  }
  /* The selected range's outer edge. */
  .outline {
    fill: none;
    stroke: var(--heat-outline);
    stroke-width: 1;
    stroke-linejoin: round;
    pointer-events: none;
  }
  /* One day: a thin accent ring instead of a band. */
  rect.ring {
    stroke: var(--accent);
    stroke-width: 1.4;
  }
  .tooltip {
    position: absolute;
    transform: translate(-50%, calc(-100% - 12px));
    pointer-events: none;
    white-space: nowrap;
    background: var(--tip-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 6px 10px;
    font-size: 12px;
    box-shadow: var(--card-shadow);
    z-index: 10;
  }
  .tooltip .value {
    color: var(--accent);
    font-size: 13px;
  }
  footer {
    font-size: 12px;
  }
  .legend {
    display: flex;
    align-items: center;
    gap: 3px;
  }
  .legend i {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 2px;
  }
</style>
