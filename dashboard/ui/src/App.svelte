<script lang="ts">
  // The dashboard shell: header (range presets, freshness, refresh), the year
  // heatmap and, for 5a, a plain-text summary of the selected range.
  //
  // Data flow: meta() once (and on refresh); series(year, metric) once per
  // (year, metric), cached until refresh; summary(from, to) whenever the
  // selected range settles. Responses that arrive after a newer request
  // are dropped.
  import { onMount, tick } from "svelte";
  import Header from "./lib/Header.svelte";
  import Heatmap from "./lib/Heatmap.svelte";
  import RangePanel from "./lib/RangePanel.svelte";
  import { errorText, getMeta, getSeries, getSummary, ready } from "./lib/api";
  import {
    type Preset,
    type Range,
    fmtAsOf,
    localToday,
    presetOf,
    presetRange,
    yearOf,
  } from "./lib/dates";
  import type { Meta, SummaryJson, YearSeries } from "./lib/types";

  const DEFAULT_METRIC = "app_usage/screen_time_secs";

  let meta = $state.raw<Meta | null>(null);
  let metaError = $state<string | null>(null);
  let selection = $state.raw<Range>(presetRange("today", localToday()));
  let year = $state(yearOf(localToday()));
  let metric = $state(DEFAULT_METRIC);

  let series = $state.raw<YearSeries | null>(null);
  let seriesLoading = $state(false);
  let seriesError = $state<string | null>(null);
  let summary = $state.raw<SummaryJson | null>(null);
  let summaryLoading = $state(false);
  let summaryError = $state<string | null>(null);
  let refreshing = $state(false);

  const seriesCache = new Map<string, YearSeries>();
  let seriesSeq = 0;
  let summarySeq = 0;

  const today = $derived(meta?.today ?? localToday());
  const preset: Preset = $derived(presetOf(selection, today));
  const maxYear = $derived(yearOf(today));
  const minYear = $derived(
    meta?.earliest_date ? Math.min(yearOf(meta.earliest_date), maxYear) : maxYear,
  );
  const asOf = $derived(meta?.last_flush_ms != null ? fmtAsOf(meta.last_flush_ms, today) : null);

  const clampYear = (y: number): number => Math.min(Math.max(y, minYear), maxYear);

  async function loadMeta() {
    try {
      meta = await getMeta();
      metaError = null;
    } catch (e) {
      metaError = errorText(e);
      return;
    }
    // Keep the chosen metric if it is still available (its collector may
    // have been turned off); otherwise screen time, otherwise the first.
    if (!meta.metrics.some((m) => m.id === metric)) {
      metric =
        meta.metrics.find((m) => m.id === DEFAULT_METRIC)?.id ?? meta.metrics[0]?.id ?? DEFAULT_METRIC;
    }
  }

  async function loadSeries() {
    const m = meta;
    if (!m?.db_found || !m.metrics.some((x) => x.id === metric)) {
      series = null;
      return;
    }
    const key = `${year}|${metric}`;
    const hit = seriesCache.get(key);
    if (hit) {
      seriesSeq++;
      series = hit;
      seriesError = null;
      seriesLoading = false;
      return;
    }
    const seq = ++seriesSeq;
    seriesLoading = true;
    try {
      const s = await getSeries(year, metric);
      seriesCache.set(key, s);
      if (seq === seriesSeq) {
        series = s;
        seriesError = null;
      }
    } catch (e) {
      if (seq === seriesSeq) seriesError = errorText(e);
    } finally {
      if (seq === seriesSeq) seriesLoading = false;
    }
  }

  async function loadSummary() {
    if (!meta?.db_found) {
      summary = null;
      return;
    }
    const seq = ++summarySeq;
    const { from, to } = selection;
    summaryLoading = true;
    try {
      const s = await getSummary(from, to);
      if (seq === summarySeq) {
        summary = s;
        summaryError = null;
      }
    } catch (e) {
      if (seq === summarySeq) summaryError = errorText(e);
    } finally {
      if (seq === summarySeq) summaryLoading = false;
    }
  }

  /** New selection. `final`: fetch the summary (not while dragging).
   *  `followYear`: show the year the range ends in (presets, date fields). */
  function select(r: Range, final: boolean, followYear: boolean) {
    selection = r;
    if (followYear) {
      const y = clampYear(yearOf(r.to));
      if (y !== year) {
        year = y;
        void loadSeries();
      }
    }
    if (final) void loadSummary();
  }

  function setYear(y: number) {
    year = clampYear(y);
    void loadSeries();
  }

  function setMetric(id: string) {
    metric = id;
    void loadSeries();
  }

  async function refresh() {
    refreshing = true;
    seriesCache.clear();
    await loadMeta();
    year = clampYear(year);
    await Promise.all([loadSeries(), loadSummary()]);
    refreshing = false;
  }

  onMount(async () => {
    await loadMeta();
    selection = presetRange("today", today);
    year = clampYear(yearOf(today));
    await Promise.all([loadSeries(), loadSummary()]);
    // Rendered (no requestAnimationFrame: it does not run while the
    // window is still hidden).
    await tick();
    void ready();
  });
</script>

<main>
  <Header
    {selection}
    {preset}
    {asOf}
    refreshing={refreshing}
    onpreset={(p) => select(presetRange(p, today), true, true)}
    oncustom={(r) => select(r, true, true)}
    onrefresh={refresh}
  />

  {#if metaError}
    <section class="card empty">
      <h2>Cannot read the database</h2>
      <p class="error">{metaError}</p>
    </section>
  {:else if meta && !meta.db_found}
    <section class="card empty">
      <h2>No data yet</h2>
      <p>
        Wrapped has not recorded anything at <code>{meta.db_path || "(no database path)"}</code>.
      </p>
      <p class="muted">
        Start Wrapped (it lives in the menu bar and records in the background). Its first flush
        lands within about ten minutes; then press refresh.
      </p>
    </section>
  {:else if meta}
    {#if meta.metrics.length === 0}
      <section class="card empty">
        <h2>All collectors are off</h2>
        <p class="muted">Turn a collector on to see its data here. Data kept from before stays hidden while it is off.</p>
      </section>
    {:else}
      <Heatmap
        {year}
        {minYear}
        {maxYear}
        {today}
        metrics={meta.metrics}
        {metric}
        {series}
        loading={seriesLoading}
        error={seriesError}
        {selection}
        onselect={(r, final) => select(r, final, false)}
        onyear={setYear}
        onmetric={setMetric}
      />
    {/if}
    <RangePanel {meta} {selection} {summary} loading={summaryLoading} error={summaryError} />
    <p class="build faint">
      <span class="num">{meta.db_path}</span> · build <span class="num">{meta.build}</span>
    </p>
  {/if}
</main>

<style>
  main {
    max-width: 1400px;
    margin: 0 auto;
    padding: 18px 24px 24px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .empty p {
    margin: 0;
  }
  code {
    font-family: var(--mono);
    font-size: 12px;
    background: var(--card-bg-strong);
    padding: 1px 5px;
    border-radius: 4px;
  }
  .build {
    margin: 0;
    font-size: 11px;
    text-align: right;
    user-select: text;
    -webkit-user-select: text;
  }
</style>
