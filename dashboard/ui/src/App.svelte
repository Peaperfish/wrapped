<script lang="ts">
  // The dashboard shell: header (range presets, freshness, refresh), the year
  // heatmap and the cards for the selected range.
  //
  // Data flow: meta() once (and on refresh); series(year, metric) once per
  // (year, metric), cached until refresh; summary(from, to) whenever the
  // selected range settles. Responses that arrive after a newer request
  // are dropped.
  import { onMount, tick } from "svelte";
  import Header from "./lib/Header.svelte";
  import Heatmap from "./lib/Heatmap.svelte";
  import { applyAppearance } from "./lib/theme";
  import Cards from "./lib/Cards.svelte";
  import DriveHealthCard from "./lib/DriveHealthCard.svelte";
  import NowTab from "./lib/now/NowTab.svelte";
  import RecapsTab from "./lib/recaps/RecapsTab.svelte";
  import StoryPlayer from "./lib/recaps/StoryPlayer.svelte";
  import { markSeen } from "./lib/recaps/seen";
  import { nowSampler } from "./lib/now/sampler.svelte";
  import SetupScreen from "./lib/SetupScreen.svelte";
  import SettingsModal from "./lib/SettingsModal.svelte";
  import {
    errorText,
    flushCollector,
    getAppNames,
    getMeta,
    getSeries,
    getSummary,
    ready,
    takeOpenRecap,
    takeOpenSettings,
  } from "./lib/api";
  import {
    type Preset,
    type Range,
    fmtAsOf,
    localToday,
    presetOf,
    presetRange,
    summaryQuery,
    yearOf,
  } from "./lib/dates";
  import type { Meta, SummaryJson, YearSeries } from "./lib/types";

  const DEFAULT_METRIC = "app_usage/screen_time_secs";

  let meta = $state.raw<Meta | null>(null);
  let metaError = $state<string | null>(null);
  let selection = $state.raw<Range>(presetRange("today", localToday()));
  let year = $state(yearOf(localToday()));

  // Overview | Now | Recaps; the last one is remembered.
  type Tab = "overview" | "now" | "recaps";
  const TAB_KEY = "wrapped.tab";
  let tab = $state<Tab>(
    (() => {
      try {
        const t = localStorage.getItem(TAB_KEY);
        return t === "now" || t === "recaps" ? t : "overview";
      } catch {
        return "overview";
      }
    })(),
  );
  function setTab(t: Tab) {
    tab = t;
    try {
      localStorage.setItem(TAB_KEY, t);
    } catch {
      /* not remembered */
    }
  }
  // Sample only while the Now tab is shown and the window is visible.
  let visible = $state(document.visibilityState === "visible");
  $effect(() => {
    const f = () => (visible = document.visibilityState === "visible");
    document.addEventListener("visibilitychange", f);
    return () => document.removeEventListener("visibilitychange", f);
  });
  const sampling = $derived(tab === "now" && visible);
  $effect(() => {
    nowSampler.setActive(sampling);
    return () => nowSampler.setActive(false);
  });
  let metric = $state(DEFAULT_METRIC);

  let series = $state.raw<YearSeries | null>(null);
  let seriesLoading = $state(false);
  let seriesError = $state<string | null>(null);
  let summary = $state.raw<SummaryJson | null>(null);
  let summaryLoading = $state(false);
  let summaryError = $state<string | null>(null);
  let refreshing = $state(false);
  let asOfFlash = $state(false);
  // Display names of bundle ids, resolved by the backend once per id.
  let names = $state.raw<Record<string, string>>({});
  const askedNames = new Set<string>();

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

  // Briefly highlight "as of" when it changes (not on the first value).
  let lastAsOf: string | null | undefined;
  let flashTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const v = asOf;
    if (lastAsOf !== undefined && v !== lastAsOf && v !== null) {
      asOfFlash = true;
      clearTimeout(flashTimer);
      flashTimer = setTimeout(() => (asOfFlash = false), 1400);
    }
    lastAsOf = v;
  });

  const clampYear = (y: number): number => Math.min(Math.max(y, minYear), maxYear);

  async function loadMeta() {
    try {
      meta = await getMeta();
      metaError = null;
      applyAppearance(meta.settings.theme, meta.settings.accent);
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

  async function reloadDriveHealth() {
    const current = (await getMeta()).drive_health;
    if (meta?.drive_health && current) meta = { ...meta, drive_health: current };
    return current;
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
    const q = summaryQuery(selection, today);
    summaryLoading = true;
    try {
      const s = await getSummary(q.from, q.to, q.vs);
      if (seq === summarySeq) {
        summary = s;
        summaryError = null;
        void loadNames(s);
      }
    } catch (e) {
      if (seq === summarySeq) summaryError = errorText(e);
    } finally {
      if (seq === summarySeq) summaryLoading = false;
    }
  }

  async function loadNames(s: SummaryJson) {
    const ids = [
      ...(s.top.find((t) => t.collector === "app_usage" && t.metric === "app_secs")?.keys ?? []).map(
        (k) => k.key,
      ),
      ...(s.records.longest_session ? [s.records.longest_session.app] : []),
    ].filter((id) => !askedNames.has(id));
    if (ids.length === 0) return;
    ids.forEach((id) => askedNames.add(id));
    try {
      names = { ...names, ...(await getAppNames(ids)) };
    } catch {
      ids.forEach((id) => askedNames.delete(id)); // shown as ids; retry next time
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

  // The recap story player (over everything); `recapVersion` makes the list
  // re-read its "New" marks and data.
  let recapOpen = $state<string | null>(null);
  let recapVersion = $state(0);
  // Bumped after a refresh or a settings change: the recap list is read again.
  let dataVersion = $state(0);
  function openRecap(id: string) {
    markSeen(id);
    recapVersion++;
    recapOpen = id;
  }
  /** A notification click or `--recap` asked for a recap. */
  async function checkOpenRecap() {
    if (meta?.setup) return;
    const id = await takeOpenRecap();
    if (!id) return;
    if (settingsOpen) await closeSettings();
    setTab("recaps");
    openRecap(id);
  }

  // Settings dialog. Focus goes back to whatever opened it (the gear).
  let settingsOpen = $state(false);
  let settingsOpener: HTMLElement | null = null;

  function openSettings(opener?: HTMLElement | null) {
    settingsOpener = opener ?? document.getElementById("settings-gear");
    settingsOpen = true;
  }

  async function closeSettings() {
    settingsOpen = false;
    await tick();
    (settingsOpener ?? document.getElementById("settings-gear"))?.focus();
  }

  /** A change was made by the collector: fetch everything again. */
  async function settingsChanged() {
    dataVersion++;
    seriesCache.clear();
    await loadMeta();
    year = clampYear(year);
    await Promise.all([loadSeries(), loadSummary()]);
  }

  /** The tray's "Settings…" leaves a flag; look for it at start and on focus. */
  async function checkOpenSettingsFlag() {
    if (meta?.setup || settingsOpen) return;
    if (await takeOpenSettings()) openSettings();
  }

  onMount(() => {
    const f = () => {
      void checkOpenSettingsFlag();
      void checkOpenRecap();
    };
    window.addEventListener("focus", f);
    return () => window.removeEventListener("focus", f);
  });

  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  /** Ask a running collector to save, wait until the last-recorded time
   *  advances (at most 1.5 s), then re-fetch everything. */
  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    try {
      const before = meta?.last_flush_ms ?? null;
      if (await flushCollector()) {
        const deadline = Date.now() + 1500;
        while (Date.now() < deadline) {
          await sleep(100);
          try {
            const m = await getMeta();
            if ((m.last_flush_ms ?? null) !== before) break;
          } catch {
            break;
          }
        }
      }
      seriesCache.clear();
      dataVersion++;
      await loadMeta();
      year = clampYear(year);
      await Promise.all([loadSeries(), loadSummary()]);
    } finally {
      refreshing = false;
    }
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
    await checkOpenSettingsFlag();
    await checkOpenRecap();
  });
</script>

{#if meta?.setup}
  <main>
    <SetupScreen
      {meta}
      ondone={async () => {
        await loadMeta();
        await Promise.all([loadSeries(), loadSummary()]);
      }}
    />
  </main>
{:else}
<main class:overview={tab === "overview"} inert={settingsOpen || recapOpen !== null}>
  <Header
    {selection}
    {preset}
    {today}
    {asOf}
    refreshing={refreshing}
    {asOfFlash}
    {tab}
    ontab={setTab}
    onpreset={(p) => select(presetRange(p, today), true, true)}
    oncustom={(r) => select(r, true, true)}
    onrefresh={refresh}
    onsettings={(el) => openSettings(el)}
  />

  {#if tab === "now"}
    <NowTab />
  {:else if tab === "recaps"}
    <RecapsTab onopen={openRecap} version={recapVersion} reload={dataVersion} />
  {:else if metaError}
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
    {#if meta.metrics.length === 0 && !meta.drive_health}
      <section class="card empty">
        <h2>All collectors are off</h2>
        <p class="muted">Turn a collector on to see its data here. Data kept from before stays hidden while it is off.</p>
      </section>
    {/if}
    <Cards {meta} {summary} {names} {preset} loading={summaryLoading} error={summaryError}>
      {#snippet activity()}
        {#if meta && meta.metrics.length > 0}
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
      {/snippet}
      {#snippet driveHealth()}
        {#if meta?.drive_health}
          <DriveHealthCard health={meta.drive_health} reload={reloadDriveHealth} paused={meta.settings.pause !== "off"} />
        {/if}
      {/snippet}
    </Cards>
    <p class="build faint">
      <span class="num">{meta.db_path}</span> · build <span class="num">{meta.build}</span>
    </p>
  {/if}
</main>
{#if recapOpen}
  <StoryPlayer id={recapOpen} onclose={() => (recapOpen = null)} />
{/if}
{#if settingsOpen && meta}
  <SettingsModal {meta} onchanged={settingsChanged} onclose={closeSettings} />
{/if}
{/if}

<style>
  main {
    max-width: 1400px;
    margin: 0 auto;
    padding: 18px 24px 24px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  main.overview {
    gap: 12px;
    padding: 12px 20px 18px;
    font-size: 13px;
    --radius: 10px;
    --card-shadow-flat: none;
    --text-faint: var(--text-dim);
    --mono: var(--font);
  }
  :global(body:has(main.overview)::before) {
    display: none;
  }
  .overview :global(:where(.card)) {
    padding: 12px 16px;
  }
  .overview :global(.card h2) {
    color: var(--text);
    font-size: 14px;
    letter-spacing: normal;
    text-transform: none;
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
