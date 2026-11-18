<script lang="ts">
  // Range presets, the selected range, data freshness and refresh.
  import DateRangePicker from "./DateRangePicker.svelte";
  import { type Preset, type Range, type Ymd, rangeButtonLabel, rangeLabel } from "./dates";

  interface Props {
    selection: Range;
    preset: Preset;
    today: Ymd;
    asOf: string | null;
    refreshing: boolean;
    asOfFlash?: boolean;
    onpreset: (p: Exclude<Preset, "custom">) => void;
    oncustom: (r: Range) => void;
    onrefresh: () => void;
  }
  let { selection, preset, today, asOf, refreshing, asOfFlash = false, onpreset, oncustom, onrefresh }: Props = $props();

  const presets: [Exclude<Preset, "custom">, string][] = [
    ["today", "Today"],
    ["week", "This week"],
    ["month", "This month"],
    ["year", "This year"],
  ];

  // Custom shows one button with the range; it opens a calendar popover and
  // stays visible while the selection is not a preset.
  let customOpen = $state(false);
  let pickerOpen = $state(false);
  let rangeBtn: HTMLButtonElement | undefined = $state();
  const showCustom = $derived(customOpen || preset === "custom");

  function closePicker() {
    pickerOpen = false;
    rangeBtn?.focus({ preventScroll: true });
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
            pickerOpen = false;
            onpreset(p);
          }}>{title}</button
        >
      {/each}
      <button
        class:active={showCustom}
        aria-pressed={showCustom}
        onclick={() => {
          customOpen = true;
          pickerOpen = true;
        }}>Custom</button
      >
    </div>

    {#if showCustom}
      <div class="dates">
        <button
          class="range-btn num"
          class:open={pickerOpen}
          aria-haspopup="dialog"
          aria-expanded={pickerOpen}
          bind:this={rangeBtn}
          onclick={() => (pickerOpen = !pickerOpen)}>{rangeButtonLabel(selection)}</button
        >
        {#if pickerOpen}
          <DateRangePicker
            {selection}
            {today}
            trigger={rangeBtn}
            onapply={(r) => {
              oncustom(r);
              closePicker();
            }}
            onclose={closePicker}
          />
        {/if}
      </div>
    {/if}

    <span class="asof muted" class:flash={asOfFlash}>
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
  /* The popover is absolutely positioned: opening it moves nothing. */
  .dates {
    position: relative;
  }
  .range-btn {
    font-size: 12px;
    white-space: nowrap;
  }
  .range-btn.open {
    border-color: rgba(var(--accent-rgb), 0.45);
  }
  .asof {
    font-size: 12px;
    white-space: nowrap;
  }
  .asof {
    border-radius: 6px;
    padding: 2px 6px;
    transition: background-color 0.9s ease, color 0.9s ease;
  }
  .asof.flash {
    background: rgba(var(--accent-rgb), 0.25);
    color: var(--text);
    transition: none;
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
