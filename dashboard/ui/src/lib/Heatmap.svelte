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
  import { fmtValue } from "./format";

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

  const unit = $derived(series?.unit ?? metrics.find((m) => m.id === metric)?.unit ?? "count");
  const daysWithData = $derived(sorted.length);
  const yearTotal = $derived(values ? values.reduce((a, b) => a + b, 0) : 0);

  const inSelection = (d: Ymd): boolean => selection.from <= d && d <= selection.to;

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
    c.future ? "var(--cell-future)" : c.level === 0 ? "var(--cell-empty)" : `var(--cell-${c.level})`;
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
          <span class="num">{daysWithData}</span> days with data{#if series?.agg === "sum" && yearTotal > 0}
            · <span class="num">{fmtValue(yearTotal, unit)}</span> in {year}{/if}
        </span>
      {/if}
      <label class="muted">
        <span class="visually-hidden">Colour by</span>
        <select value={metric} onchange={(e) => onmetric(e.currentTarget.value)}>
          {#each metrics as m (m.id)}
            <option value={m.id}>{m.title}</option>
          {/each}
        </select>
      </label>
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
          class:selected={inSelection(c.date)}
          class:today={c.date === today}
        />
      {/each}
    </svg>

    {#if hover}
      <div class="tooltip" style="left: {hover.x}px; top: {hover.y}px">
        <div>{fmtDay(hover.cell.date)}</div>
        <div class="num value">
          {hover.cell.future ? "—" : values ? fmtValue(hover.cell.value, unit) : "…"}
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
  rect.selected {
    stroke: var(--cell-selected);
    stroke-width: 1.4;
  }
  .tooltip {
    position: absolute;
    transform: translate(-50%, calc(-100% - 12px));
    pointer-events: none;
    white-space: nowrap;
    background: rgba(18, 21, 30, 0.95);
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
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
</style>
