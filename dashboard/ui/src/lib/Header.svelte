<script lang="ts">
  // Range presets, the selected range, data freshness and refresh.
  import { type Preset, type Range, type Ymd, isValidYmd, ordered, rangeLabel } from "./dates";

  interface Props {
    selection: Range;
    preset: Preset;
    asOf: string | null;
    refreshing: boolean;
    onpreset: (p: Exclude<Preset, "custom">) => void;
    oncustom: (r: Range) => void;
    onrefresh: () => void;
  }
  let { selection, preset, asOf, refreshing, onpreset, oncustom, onrefresh }: Props = $props();

  const presets: [Exclude<Preset, "custom">, string][] = [
    ["today", "Today"],
    ["week", "This week"],
    ["month", "This month"],
    ["year", "This year"],
  ];

  // Custom shows two date fields; it opens on click and stays open while
  // the selection is not a preset.
  let customOpen = $state(false);
  const showCustom = $derived(customOpen || preset === "custom");

  function setDate(which: "from" | "to", value: Ymd) {
    if (!isValidYmd(value)) return;
    const other = which === "from" ? selection.to : selection.from;
    oncustom(ordered(value, other));
  }
</script>

<header class="bar">
  <div class="range">
    <span class="brand">Wrapped</span>
    <span class="label">{rangeLabel(selection)}</span>
  </div>

  <div class="right">
    <div class="presets" role="group" aria-label="Range">
      {#each presets as [p, title] (p)}
        <button
          class:active={preset === p && !customOpen}
          aria-pressed={preset === p && !customOpen}
          onclick={() => {
            customOpen = false;
            onpreset(p);
          }}>{title}</button
        >
      {/each}
      <button
        class:active={showCustom}
        aria-pressed={showCustom}
        onclick={() => (customOpen = true)}>Custom</button
      >
    </div>

    {#if showCustom}
      <div class="dates">
        <input
          type="date"
          aria-label="From"
          value={selection.from}
          onchange={(e) => setDate("from", e.currentTarget.value)}
        />
        <span class="faint">–</span>
        <input
          type="date"
          aria-label="To"
          value={selection.to}
          onchange={(e) => setDate("to", e.currentTarget.value)}
        />
      </div>
    {/if}

    <span class="asof muted">
      {#if asOf}as of <span class="num">{asOf}</span>{:else}no data yet{/if}
    </span>
    <button class="refresh" class:spinning={refreshing} aria-label="Refresh" title="Refresh" onclick={onrefresh}>
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
        <path
          d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 2.5v3h-3"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
    </button>
  </div>
</header>

<style>
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
  }
  .range {
    display: flex;
    align-items: baseline;
    gap: 14px;
    min-width: 0;
  }
  .brand {
    font-weight: 700;
    letter-spacing: 0.02em;
    color: var(--accent);
  }
  .label {
    font-size: 20px;
    font-weight: 600;
    white-space: nowrap;
  }
  .right {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .presets {
    display: flex;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: 10px;
    padding: 3px;
    gap: 2px;
  }
  .presets button {
    background: transparent;
    border-color: transparent;
    color: var(--text-dim);
    padding: 4px 10px;
  }
  .presets button.active {
    background: rgba(var(--accent-rgb), 0.16);
    border-color: rgba(var(--accent-rgb), 0.35);
    color: var(--text);
  }
  .dates {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .asof {
    font-size: 12px;
    white-space: nowrap;
  }
  .refresh {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    padding: 0;
  }
  .refresh.spinning svg {
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
