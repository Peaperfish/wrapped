<script lang="ts">
  // Overview sections for the selected range, gated by enabled collectors.
  import type { Snippet } from "svelte";
  import BarList, { type BarItem } from "./BarList.svelte";
  import BigNumber from "./BigNumber.svelte";
  import Card from "./Card.svelte";
  import HourBars from "./HourBars.svelte";
  import Ring from "./Ring.svelte";
  import { AI_TOOLS, modelLabel } from "./aiNames";
  import { fmtBytes, fmtSecs, fmtTokens, fmtValue } from "./format";
  import { type Preset, fmtDay, rangeLabel } from "./dates";
  import type { Meta, StreakJson, SummaryJson, TopJson } from "./types";

  interface Props {
    meta: Meta;
    summary: SummaryJson | null;
    /** Display names of bundle ids (missing ids are shown as they are). */
    names: Record<string, string>;
    preset: Preset;
    loading: boolean;
    error: string | null;
    activity: Snippet;
    driveHealth: Snippet;
  }
  let { meta, summary, names, preset, loading, error, activity, driveHealth }: Props = $props();

  const appName = (id: string): string => names[id] ?? id;
  // Today is part of a day compared with all of yesterday.
  const soFar = $derived(preset === "today");
  // The comparison period has data on fewer than half as many days: no % change.
  const thin = $derived(!!summary && summary.previous_data_days * 2 < summary.data_days);

  const on = $derived(new Set(meta.collectors.filter((c) => c.enabled).map((c) => c.name)));

  const total = (c: string, m: string): number | undefined =>
    summary?.totals.find((t) => t.collector === c && t.metric === m && t.key === "")?.value;
  const cmp = (c: string, m: string) =>
    summary?.comparisons.find((x) => x.collector === c && x.metric === m);
  const top = (c: string, m: string): TopJson | undefined =>
    summary?.top.find((t) => t.collector === c && t.metric === m);
  const items = (t: TopJson | undefined, n: number, label = (k: string) => k): BarItem[] =>
    (t?.keys ?? []).slice(0, n).map((k) => ({
      label: label(k.key),
      value: k.value,
      text: fmtValue(k.value, t!.unit),
      title: `${k.key}: ${fmtValue(k.value, t!.unit)}`,
    }));

  const screen = $derived(total("app_usage", "screen_time_secs"));
  // Away while awake: every reason but sleep. (An idle period that turns into
  // sleep is still keyed "idle", so this can overcount a little.)
  const away = $derived.by(() => {
    const t = top("active_idle", "away_secs_by_reason");
    if (!t) return total("active_idle", "away_secs") === undefined ? undefined : 0;
    return t.keys.filter((k) => k.key !== "sleep").reduce((a, k) => a + k.value, 0);
  });
  const activeDays = $derived(
    (summary?.series.find((s) => s.collector === "app_usage" && s.metric === "screen_time_secs")?.days ?? []).filter(
      (v) => v > 0,
    ).length,
  );
  const hours = $derived(
    summary?.time_of_day.find((p) => p.collector === "app_usage")?.hours ?? [],
  );
  const apps = $derived(items(top("app_usage", "app_secs"), 8, appName));
  const commits = $derived(total("git", "commits"));
  const repos = $derived(items(top("git", "commits_by_repo"), 8));
  const shots = $derived(total("photos", "shots"));
  const cameras = $derived(items(top("photos", "shots_by_camera"), 5));
  const lenses = $derived(items(top("photos", "shots_by_lens"), 5));
  const TOOLS = AI_TOOLS;
  const tokens = $derived(total("ai_usage", "tokens"));
  const aiRequests = $derived(total("ai_usage", "requests") ?? 0);
  const aiDays = $derived(
    (summary?.series.find((s) => s.collector === "ai_usage" && s.metric === "tokens")?.days ?? []).filter(
      (v) => v > 0,
    ).length,
  );
  const kind = (k: string): number =>
    top("ai_usage", "tokens_by_kind")?.keys.find((x) => x.key === k)?.value ?? 0;
  const aiTools = $derived(
    items(top("ai_usage", "tokens_by_tool"), 6, (k) => TOOLS[k] ?? k).map((i) => ({
      ...i,
      text: fmtTokens(i.value),
    })),
  );
  const aiModels = $derived(
    items(top("ai_usage", "tokens_by_model"), 6, modelLabel).map((i) => ({ ...i, text: fmtTokens(i.value) })),
  );
  const aiProjects = $derived(
    items(top("ai_usage", "tokens_by_project"), 6).map((i) => ({ ...i, text: fmtTokens(i.value) })),
  );
  // Tools that log requests but no token counts (Cursor CLI).
  const noTokenTools = $derived(
    (top("ai_usage", "requests_by_tool")?.keys ?? [])
      .filter((k) => !aiTools.some((t) => t.label === (TOOLS[k.key] ?? k.key)))
      .map((k) => TOOLS[k.key] ?? k.key),
  );
  const rec = $derived(summary?.records);
  const noRecords = $derived(
    !rec || (!rec.longest_session && !rec.latest_night && !rec.active_streak && !rec.commit_streak),
  );

  const LEVELS = ["normal", "warning", "critical"];
  const pressure = $derived.by(() => {
    const t = top("disk_memory", "pressure_secs_by_level");
    if (!t) return [];
    const by = new Map(t.keys.map((k) => [k.key, k.value]));
    return LEVELS.filter((l) => by.has(l)).map((l) => ({
      label: l[0]!.toUpperCase() + l.slice(1),
      value: by.get(l)!,
      text: fmtSecs(by.get(l)!),
    }));
  });

  const day = (iso: string): string => fmtDay(iso.slice(0, 10));
  const streakDates = (s: StreakJson): string =>
    s.from === s.to ? fmtDay(s.from) : `${fmtDay(s.from, false)} – ${fmtDay(s.to)}`;
  const plural = (n: number, one: string, many = `${one}s`): string => `${n} ${n === 1 ? one : many}`;
  const vsLabel = $derived(
    summary ? `vs ${rangeLabel({ from: summary.previous.from, to: summary.previous.to })}` : "",
  );

  const diskMetrics = [
    ["bytes_read", "Read"],
    ["bytes_written", "Written"],
    ["swap_in_bytes", "Swapped in"],
    ["swap_out_bytes", "Swapped out"],
  ] as const;
  const recordedDiskMetrics = $derived(diskMetrics.filter(([m]) => total("disk_memory", m) !== undefined));
  const showDisk = $derived(!!summary && !error && on.has("disk_memory"));
  const diskOnly = $derived(recordedDiskMetrics.length > 0 && total("disk_memory", "swap_in_bytes") === undefined);
</script>

{#if error || !summary}
  <section class="card"><p class="line {error ? 'error' : 'muted'}" role="status">{error ?? "Loading…"}</p></section>
{:else if on.has("app_usage") || on.has("active_idle")}
  <section class="card split key-stats" class:loading aria-label="Key stats">
    {#if on.has("app_usage")}
      <Card title="Screen time" subtitle={vsLabel} size="M" empty={screen === undefined}>
        <BigNumber {thin} {soFar} value={fmtSecs(screen ?? 0)} cmp={cmp("app_usage", "screen_time_secs")} />
        <p class="line muted">
          <span class="num">{activeDays > 0 ? fmtSecs((screen ?? 0) / activeDays) : "–"}</span> per active day
          <span class="faint">({plural(activeDays, "day")})</span>
        </p>
      </Card>
    {/if}

    {#if on.has("active_idle")}
      <Card title="Away (awake)" size="M" empty={away === undefined && screen === undefined}>
        {@const s = screen ?? 0}
        {@const a = away ?? 0}
        <div class="ringrow">
          <Ring
            --ring-size="78px"
            fraction={s + a > 0 ? s / (s + a) : 0}
            center={s + a > 0 ? `${Math.round((s / (s + a)) * 100)}%` : "–"}
            title="Screen time {fmtSecs(s)}, away while awake {fmtSecs(a)}"
          />
          <dl>
            <dt class="muted"><i class="sw on"></i>At screen</dt>
            <dd class="num">{fmtSecs(s)}</dd>
            <dt class="muted"><i class="sw"></i>Away (awake)</dt>
            <dd class="num">{fmtSecs(a)}</dd>
          </dl>
        </div>
      </Card>
    {/if}
  </section>
{/if}

{@render activity()}

{#if summary && !error && on.has("app_usage")}
  <div class="card split app-activity" class:loading>
    <Card title="Top Apps" subtitle="By screen time" size="M" empty={apps.length === 0}>
      <BarList items={apps} compact />
    </Card>
    <Card title="Time of Day" subtitle="Screen time by local hour" size="M" empty={hours.every((h) => h === 0)}>
      <HourBars {hours} format={fmtSecs} />
    </Card>
  </div>
{/if}

{#if showDisk || meta.drive_health}
  <section class="card storage" aria-labelledby="storage-heading">
    <header class="storage-heading">
      <h2 id="storage-heading">{showDisk && !diskOnly ? "Storage & Memory" : "Storage"}</h2>
      {#if showDisk}<p class="line muted">Includes background system activity</p>{/if}
    </header>
    <div class="split">
      {#if showDisk}
        <Card
          title={diskOnly ? "Disk" : "Disk & Swap"}
          size="M"
          empty={recordedDiskMetrics.length === 0}
        >
          <div class="quad">
            {#each recordedDiskMetrics as [m, label] (m)}
              <BigNumber {thin} small {label} value={fmtBytes(total("disk_memory", m) ?? 0)} {soFar} cmp={cmp("disk_memory", m)} />
            {/each}
          </div>
        </Card>

        {#if !diskOnly}
          <Card
            title="Memory pressure"
            size="M"
            empty={pressure.length === 0}
          >
            <BarList items={pressure} />
            <p class="line muted">
              Compressed memory: peak
              <span class="num">{fmtBytes(total("disk_memory", "compressed_peak_bytes") ?? 0)}</span>, average
              <span class="num">{fmtBytes(total("disk_memory", "compressed_avg_bytes") ?? 0)}</span>
            </p>
          </Card>
        {/if}
      {/if}
      {@render driveHealth()}
    </div>
  </section>
{/if}

{#if summary && !error && (on.has("app_usage") || on.has("git") || on.has("photos") || on.has("ai_usage"))}
  <div class="grid" class:loading>
    {#if on.has("app_usage") || on.has("git")}
      <Card title="Records" size="M" empty={noRecords}>
        <dl class="records">
          {#if rec?.longest_session}
            <dt class="muted">Longest session</dt>
            <dd>
              <span class="num">{fmtSecs(rec.longest_session.secs)}</span>
              <span class="faint"
                >{appName(rec.longest_session.app)} · {day(rec.longest_session.start.local)}</span
              >
            </dd>
          {/if}
          {#if rec?.latest_night}
            <dt class="muted">Latest night</dt>
            <dd>
              <span class="num">{rec.latest_night.at.local.slice(11, 16)}</span>
              <span class="faint">{fmtDay(rec.latest_night.night)}</span>
            </dd>
          {/if}
          {#if rec?.active_streak}
            <dt class="muted">Active streak</dt>
            <dd>
              <span class="num">{plural(rec.active_streak.days, "day")}</span>
              <span class="faint">{streakDates(rec.active_streak)}</span>
            </dd>
          {/if}
          {#if rec?.commit_streak}
            <dt class="muted">Commit streak</dt>
            <dd>
              <span class="num">{plural(rec.commit_streak.days, "day")}</span>
              <span class="faint">{streakDates(rec.commit_streak)}</span>
            </dd>
          {/if}
        </dl>
      </Card>
    {/if}

    {#if on.has("git")}
      <Card title="Git" size="M" empty={commits === undefined}>
        <BigNumber {thin} value={fmtValue(commits ?? 0, "count")} label="Commits" {soFar} cmp={cmp("git", "commits")} />
        <BarList items={repos} />
        {#if rec?.commit_streak}
          <p class="line muted">
            Commit streak <span class="num">{plural(rec.commit_streak.days, "day")}</span>
            <span class="faint">{streakDates(rec.commit_streak)}</span>
          </p>
        {/if}
      </Card>
    {/if}

    {#if on.has("photos")}
      <Card title="Photos" size="M" empty={shots === undefined}>
        <BigNumber {thin} value={fmtValue(shots ?? 0, "count")} label="Shots" {soFar} cmp={cmp("photos", "shots")} />
        <div class="two">
          <div>
            <h3 class="muted">Cameras</h3>
            <BarList items={cameras} />
          </div>
          <div>
            <h3 class="muted">Lenses</h3>
            <BarList items={lenses} />
          </div>
        </div>
      </Card>
    {/if}

    {#if on.has("ai_usage")}
      <Card
        title="AI usage"
        subtitle="Input + output tokens; cache shown separately"
        size="L"
        empty={(tokens ?? 0) === 0 && aiRequests === 0}
      >
        <div class="ai">
          <div class="aitop">
            <BigNumber {thin}
              value={fmtTokens(tokens ?? 0)}
              label="Tokens"
              {soFar}
              cmp={cmp("ai_usage", "tokens")}
            />
            <dl class="cache">
              <dt class="muted">Input</dt>
              <dd class="num" title="{fmtValue(kind('input'), 'count')} tokens">{fmtTokens(kind("input"))}</dd>
              <dt class="muted">Output</dt>
              <dd class="num" title="{fmtValue(kind('output'), 'count')} tokens">{fmtTokens(kind("output"))}</dd>
              <dt class="muted">Cache read</dt>
              <dd class="num" title="{fmtValue(kind('cache_read'), 'count')} tokens">{fmtTokens(kind("cache_read"))}</dd>
              <dt class="muted">Cache write</dt>
              <dd class="num" title="{fmtValue(kind('cache_write'), 'count')} tokens">{fmtTokens(kind("cache_write"))}</dd>
            </dl>
            <p class="line muted">
              <span class="num">{fmtValue(aiRequests, "count")}</span> requests,
              <span class="num">{fmtValue(total("ai_usage", "sessions") ?? 0, "count")}</span> sessions
              <span class="faint">({plural(aiDays, "active day")})</span>
            </p>
          </div>
          <div class="three">
            <div>
              <h3 class="muted">By tool</h3>
              <BarList items={aiTools} />
            </div>
            <div>
              <h3 class="muted">By model</h3>
              <BarList items={aiModels} />
            </div>
            <div>
              <h3 class="muted">By project</h3>
              <BarList items={aiProjects} />
            </div>
          </div>
          {#if noTokenTools.length > 0}
            <p class="line faint">{noTokenTools.join(", ")}: sessions only (no token counts on disk).</p>
          {/if}
        </div>
      </Card>
    {/if}
  </div>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
    gap: 12px;
  }
  .grid > :global(.card:last-child:nth-child(odd)) {
    grid-column: 1 / -1;
  }
  .split {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: minmax(0, 1fr);
  }
  .card.split {
    padding-block: 0;
  }
  .split > :global(.card) {
    grid-column: auto;
    border: 0;
    border-radius: 0;
    background: none;
    padding: 12px 0;
    gap: 8px;
  }
  .split > :global(.card:not(:last-child)) {
    padding-inline-end: 16px;
  }
  .split > :global(.card + .card),
  .split > :global(.card + .drive-health) {
    border-inline-start: 1px solid var(--divider);
    padding-inline-start: 16px;
  }
  .split :global(.none) {
    padding: 0;
    text-align: left;
    font-size: 12px;
  }
  .key-stats :global(.value) {
    font-size: 27px;
    letter-spacing: -0.035em;
  }
  .key-stats > :global(.card) {
    justify-content: center;
  }
  .key-stats :global(.body) {
    flex: initial;
    gap: 5px;
  }
  .storage-heading {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: 4px 16px;
    padding-bottom: 9px;
    border-bottom: 1px solid var(--divider);
  }
  .storage .split > :global(.card) {
    padding-block: 12px 0;
  }
  .storage .split :global(.card h2) {
    font-size: 13px;
  }
  .loading {
    opacity: 0.7;
    transition: opacity 0.2s;
  }
  @media (max-width: 760px) {
    .split {
      grid-auto-flow: row;
    }
    .split > :global(.card:not(:last-child)),
    .split > :global(.card + .card),
    .split > :global(.card + .drive-health) {
      padding-inline: 0;
      border-inline: 0;
    }
    .split > :global(.card + .card),
    .split > :global(.card + .drive-health) {
      border-top: 1px solid var(--divider);
    }
    .storage .split > :global(.card:not(:last-child)) {
      padding-bottom: 12px;
    }
    .grid > :global(.card) {
      grid-column: 1 / -1;
    }
  }
  .line {
    margin: 0;
    font-size: 12px;
  }
  .ringrow {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .ringrow dl {
    gap: 4px 14px;
    font-size: 12px;
  }
  /* Colour key matching the ring: accent = at screen, grey = away. */
  .sw {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-right: 7px;
    background: var(--cell-empty);
    box-shadow: inset 0 0 0 1px var(--card-border);
  }
  .sw.on {
    background: var(--accent);
    box-shadow: none;
  }
  dl {
    margin: 0;
    display: grid;
    grid-template-columns: auto auto;
    gap: 4px 14px;
    font-size: 13px;
  }
  dd {
    margin: 0;
    text-align: right;
  }
  .records {
    grid-template-columns: auto 1fr;
    gap: 6px 16px;
  }
  .records dd {
    text-align: left;
    display: flex;
    gap: 10px;
    align-items: baseline;
    flex-wrap: wrap;
  }
  .records .num {
    font-size: 16px;
    font-weight: 600;
  }
  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 18px;
  }
  .two > div {
    min-width: 0;
  }
  h3 {
    margin: 0 0 8px;
    font-size: 12px;
    font-weight: 600;
  }
  .ai {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .aitop {
    display: flex;
    gap: 28px;
    align-items: center;
    flex-wrap: wrap;
  }
  .cache {
    font-size: 13px;
  }
  .three {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 18px;
  }
  .three > div {
    min-width: 0;
  }
  .quad {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px 14px;
  }
</style>
