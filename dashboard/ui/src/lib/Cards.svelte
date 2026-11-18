<script lang="ts">
  // The cards for the selected range, in a 12-column grid. Cards of
  // collectors that are off are not shown.
  import BarList, { type BarItem } from "./BarList.svelte";
  import BigNumber from "./BigNumber.svelte";
  import Card from "./Card.svelte";
  import HourBars from "./HourBars.svelte";
  import Ring from "./Ring.svelte";
  import { fmtBytes, fmtSecs, appName, fmtValue } from "./format";
  import { fmtDay, rangeLabel } from "./dates";
  import type { Meta, StreakJson, SummaryJson, TopJson } from "./types";

  interface Props {
    meta: Meta;
    summary: SummaryJson | null;
    loading: boolean;
    error: string | null;
  }
  let { meta, summary, loading, error }: Props = $props();

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
  const away = $derived(total("active_idle", "away_secs"));
  const awayCount = $derived(total("active_idle", "away_periods"));
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
</script>

{#if error}
  <section class="card"><p class="error">{error}</p></section>
{:else if !summary}
  <section class="card"><p class="muted">Loading…</p></section>
{:else}
  <div class="grid" class:loading>
    {#if on.has("app_usage")}
      <Card title="Screen time" subtitle={vsLabel} size="S" empty={screen === undefined}>
        <BigNumber value={fmtSecs(screen ?? 0)} cmp={cmp("app_usage", "screen_time_secs")} />
        <p class="line muted">
          <span class="num">{activeDays > 0 ? fmtSecs((screen ?? 0) / activeDays) : "–"}</span> per active day
          <span class="faint">({plural(activeDays, "day")})</span>
        </p>
      </Card>
    {/if}

    {#if on.has("active_idle")}
      <Card title="Away" subtitle="Screen time vs time away" size="S" empty={away === undefined && screen === undefined}>
        {@const s = screen ?? 0}
        {@const a = away ?? 0}
        <div class="ringrow">
          <Ring
            fraction={s + a > 0 ? s / (s + a) : 0}
            center={s + a > 0 ? `${Math.round((s / (s + a)) * 100)}%` : "–"}
            caption="at screen"
            title="Screen time {fmtSecs(s)}, away {fmtSecs(a)}"
          />
          <dl>
            <dt class="muted">Screen</dt>
            <dd class="num">{fmtSecs(s)}</dd>
            <dt class="muted">Away</dt>
            <dd class="num">{fmtSecs(a)}</dd>
            <dt class="muted">Times away</dt>
            <dd class="num">{awayCount ?? 0}</dd>
          </dl>
        </div>
      </Card>
    {/if}

    {#if on.has("app_usage")}
      <Card title="Time of day" subtitle="Screen time by local hour" size="M" empty={hours.every((h) => h === 0)}>
        <HourBars {hours} format={fmtSecs} />
      </Card>

      <Card title="Top apps" subtitle="By screen time" size="M" empty={apps.length === 0}>
        <BarList items={apps} />
      </Card>
    {/if}

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
        <BigNumber value={String(commits ?? 0)} label="Commits" cmp={cmp("git", "commits")} />
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
        <BigNumber value={String(shots ?? 0)} label="Shots" cmp={cmp("photos", "shots")} />
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

    {#if on.has("disk_memory")}
      <Card
        title="Disk & Swap"
        subtitle="Includes background system activity"
        size="M"
        empty={!diskMetrics.some(([m]) => total("disk_memory", m) !== undefined)}
      >
        <div class="quad">
          {#each diskMetrics as [m, label] (m)}
            <BigNumber small {label} value={fmtBytes(total("disk_memory", m) ?? 0)} cmp={cmp("disk_memory", m)} />
          {/each}
        </div>
      </Card>

      <Card
        title="Memory pressure"
        subtitle="Includes background system activity"
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
  </div>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
    gap: 16px;
    transition: opacity 0.2s;
  }
  .grid.loading {
    opacity: 0.7;
  }
  .line {
    margin: 0;
    font-size: 13px;
  }
  .ringrow {
    display: flex;
    align-items: center;
    gap: 14px;
    flex-wrap: wrap;
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
    gap: 10px 18px;
    font-size: 14px;
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
  .quad {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px 18px;
  }
</style>
