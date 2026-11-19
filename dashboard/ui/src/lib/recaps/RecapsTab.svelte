<script lang="ts">
  // The Recaps tab: every available recap, grouped Year / Month / Week,
  // newest first, each with its period and one or two headline numbers.
  // Recaps not opened yet carry a "New" mark (seen.ts).
  import { errorText, getRecapList } from "../api";
  import type { RecapEntryJson } from "../types";
  import { hm } from "./slides";
  import { isSeen } from "./seen";

  interface Props {
    onopen: (id: string) => void;
    /** Bumped when a recap was opened: re-read the "New" marks. */
    version: number;
    /** Bumped after a refresh: read the list again. */
    reload: number;
  }
  let { onopen, version, reload }: Props = $props();

  let list = $state.raw<RecapEntryJson[] | null>(null);
  let error = $state<string | null>(null);

  async function load() {
    try {
      list = await getRecapList();
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }
  $effect(() => {
    void reload;
    void load();
  });

  const groups = $derived(
    (["year", "month", "week"] as const)
      .map((k) => ({
        kind: k,
        title: k === "year" ? "Years" : k === "month" ? "Months" : "Weeks",
        items: (list ?? []).filter((e) => e.kind === k),
      }))
      .filter((g) => g.items.length > 0),
  );
  // Re-read seen marks when the parent bumps `version`.
  const seen = $derived.by(() => {
    void version;
    return new Set((list ?? []).filter((e) => isSeen(e.id)).map((e) => e.id));
  });
</script>

{#if error}
  <section class="card empty">
    <h2>Cannot read recaps</h2>
    <p class="error">{error}</p>
  </section>
{:else if list && list.length === 0}
  <section class="card empty">
    <h2>No recaps yet</h2>
    <p class="muted">
      A recap appears when a week, month or year is over and has enough days with data: 2 for a week,
      7 for a month, 30 for a year. This year's recap appears from 1 December.
    </p>
  </section>
{:else if list}
  {#each groups as g (g.kind)}
    <section class="group">
      <h2 class="muted">{g.title}</h2>
      <div class="grid">
        {#each g.items as e (e.id)}
          <button class="item card {e.kind}" onclick={() => onopen(e.id)}>
            <span class="top">
              <span class="label">{e.label}</span>
              {#if !seen.has(e.id)}<span class="new">New</span>{/if}
            </span>
            <span class="nums">
              {#if e.screen_secs}
                <span><span class="num big">{hm(e.screen_secs)}</span> <span class="muted">screen time</span></span>
              {/if}
              <span><span class="num" class:big={!e.screen_secs}>{e.active_days}</span> <span class="muted">active {e.active_days === 1 ? "day" : "days"}</span></span>
            </span>
            {#if e.in_progress}<span class="sofar faint">so far · updates daily</span>{/if}
          </button>
        {/each}
      </div>
    </section>
  {/each}
{:else}
  <section class="card empty"><p class="muted">Loading…</p></section>
{/if}

<style>
  .empty {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .empty p {
    margin: 0;
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .group h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 12px;
  }
  .item {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
    text-align: left;
    padding: 14px 16px;
    min-width: 0;
    position: relative;
    overflow: hidden;
  }
  .item::before {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 4px;
    background: var(--accent);
    opacity: 0.5;
  }
  .item.year::before {
    opacity: 1;
  }
  .item:hover {
    border-color: rgba(var(--accent-rgb), 0.5);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .label {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .new {
    flex: none;
    font-size: 11px;
    font-weight: 700;
    padding: 1px 7px;
    border-radius: 9px;
    background: rgba(var(--accent-rgb), 0.9);
    color: var(--accent-ink);
  }
  .nums {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    font-size: 13px;
  }
  .big {
    font-size: 18px;
    font-weight: 600;
    color: var(--accent);
  }
  .sofar {
    font-size: 12px;
  }
</style>
