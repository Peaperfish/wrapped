<script lang="ts">
  // Temporary (Milestone 5a): the selected range's summary as plain text.
  // The real cards replace this in 5b.
  import type { Meta, SummaryJson, TotalJson } from "./types";
  import { type Range, daysIn, rangeLabel } from "./dates";
  import { fmtPct, fmtSecs, fmtValue } from "./format";

  interface Props {
    meta: Meta;
    selection: Range;
    summary: SummaryJson | null;
    loading: boolean;
    error: string | null;
  }
  let { meta, selection, summary, loading, error }: Props = $props();

  // Titles of metrics without keys come with the summary's series; keyed
  // ones with its top lists.
  const titles = $derived.by(() => {
    const t = new Map<string, string>();
    for (const s of summary?.series ?? []) t.set(`${s.collector}/${s.metric}`, s.title);
    for (const s of summary?.top ?? []) t.set(`${s.collector}/${s.metric}`, s.title);
    return t;
  });

  interface Group {
    name: string;
    title: string;
    totals: TotalJson[];
    pct: Map<string, number | null>;
  }
  const groups: Group[] = $derived.by(() => {
    if (!summary) return [];
    const pct = new Map(summary.comparisons.map((c) => [`${c.collector}/${c.metric}`, c.pct]));
    return meta.collectors
      .filter((c) => c.enabled)
      .map((c) => ({
        name: c.name,
        title: c.title,
        totals: summary.totals.filter((t) => t.collector === c.name && t.key === ""),
        pct,
      }));
  });

  const tops = $derived((summary?.top ?? []).filter((t) => t.keys.length > 0));
  const hidden = $derived(meta.collectors.filter((c) => !c.enabled).map((c) => c.title));
  const empty = $derived(!!summary && summary.totals.length === 0);
</script>

<section class="card panel" class:loading>
  <header>
    <h2>Selected range</h2>
    <span class="faint note">plain-text preview · cards arrive in 5b</span>
  </header>

  <p class="range">
    <strong>{rangeLabel(selection)}</strong>
    <span class="muted"
      ><span class="num">{daysIn(selection)}</span>
      {daysIn(selection) === 1 ? "day" : "days"}{#if summary}
        · compared with {rangeLabel({ from: summary.previous.from, to: summary.previous.to })}{/if}</span
    >
  </p>

  {#if error}
    <p class="error">{error}</p>
  {:else if !summary}
    <p class="muted">Loading…</p>
  {:else if empty}
    <p class="muted">No data recorded in this range.</p>
  {:else}
    <div class="cols">
      {#each groups as g (g.name)}
        <div class="group">
          <h3>{g.title}</h3>
          {#if g.totals.length === 0}
            <p class="faint">no data</p>
          {:else}
            <dl>
              {#each g.totals as t (t.metric)}
                {@const p = g.pct.get(`${t.collector}/${t.metric}`)}
                <dt>{titles.get(`${t.collector}/${t.metric}`) ?? t.metric}</dt>
                <dd>
                  <span class="num">{fmtValue(t.value, t.unit)}</span>
                  {#if p !== undefined}<span class="num faint">{fmtPct(p)}</span>{/if}
                </dd>
              {/each}
            </dl>
          {/if}
        </div>
      {/each}
    </div>

    {#if tops.length > 0}
      <div class="tops">
        {#each tops as t (`${t.collector}/${t.metric}`)}
          <p>
            <span class="muted">{t.title}:</span>
            {#each t.keys as k, i (k.key)}{i > 0 ? " · " : " "}{k.key}
              <span class="num faint">{fmtValue(k.value, t.unit)}</span>{/each}
          </p>
        {/each}
      </div>
    {/if}

    <p class="records">
      <span class="muted">Records:</span>
      {#if summary.records.longest_session}
        longest session <span class="num">{fmtSecs(summary.records.longest_session.secs)}</span>
        ({summary.records.longest_session.app}) ·
      {/if}
      {#if summary.records.latest_night}
        latest night <span class="num">{summary.records.latest_night.at.local.slice(11, 16)}</span>
        on {summary.records.latest_night.night} ·
      {/if}
      {#if summary.records.active_streak}
        active streak <span class="num">{summary.records.active_streak.days}</span> d ·
      {/if}
      {#if summary.records.commit_streak}
        commit streak <span class="num">{summary.records.commit_streak.days}</span> d
      {/if}
      {#if !summary.records.longest_session && !summary.records.latest_night && !summary.records.active_streak && !summary.records.commit_streak}
        <span class="faint">none</span>
      {/if}
    </p>
  {/if}

  {#if hidden.length > 0}
    <p class="faint hidden">Off (hidden): {hidden.join(", ")}</p>
  {/if}
</section>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: 10px;
    transition: opacity 0.2s;
  }
  .panel.loading {
    opacity: 0.7;
  }
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }
  .note {
    font-size: 12px;
  }
  p {
    margin: 0;
  }
  .range strong {
    font-weight: 600;
    margin-right: 8px;
  }
  .cols {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 14px 28px;
  }
  h3 {
    margin: 0 0 4px;
    font-size: 13px;
    font-weight: 600;
  }
  dl {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 2px 12px;
    margin: 0;
  }
  dt {
    color: var(--text-dim);
  }
  dd {
    margin: 0;
    text-align: right;
    display: flex;
    gap: 10px;
    justify-content: flex-end;
  }
  dd .faint {
    min-width: 5.5em;
    text-align: right;
  }
  .tops,
  .records,
  .hidden {
    font-size: 13px;
    border-top: 1px solid var(--divider);
    padding-top: 8px;
  }
  .tops p + p {
    margin-top: 2px;
  }
</style>
