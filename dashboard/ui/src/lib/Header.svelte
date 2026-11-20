<script lang="ts">
  // Shared header (Wrapped on the left, plain-text tab switcher centred,
  // Settings on the right) and the tab-specific title / actions row below it.
  import DateRangePicker from "./DateRangePicker.svelte";
  import { type Preset, type Range, type Ymd, rangeButtonLabel, rangeLabel } from "./dates";

  type Tab = "overview" | "now" | "recaps";

  interface Props {
    selection: Range;
    preset: Preset;
    today: Ymd;
    asOf: string | null;
    refreshing: boolean;
    asOfFlash?: boolean;
    tab: Tab;
    ontab: (t: Tab) => void;
    onpreset: (p: Exclude<Preset, "custom">) => void;
    oncustom: (r: Range) => void;
    onrefresh: () => void;
    /** Open Settings; the gear button is where focus returns on close. */
    onsettings: (opener: HTMLElement) => void;
  }
  let {
    selection,
    preset,
    today,
    asOf,
    refreshing,
    asOfFlash = false,
    tab,
    ontab,
    onpreset,
    oncustom,
    onrefresh,
    onsettings,
  }: Props = $props();

  const tabs: [Tab, string][] = [
    ["overview", "Overview"],
    ["now", "Now"],
    ["recaps", "Recaps"],
  ];

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

  function tabKey(e: KeyboardEvent) {
    const i = tabs.findIndex(([t]) => t === tab);
    let j = i;
    if (e.key === "ArrowRight") j = (i + 1) % tabs.length;
    else if (e.key === "ArrowLeft") j = (i - 1 + tabs.length) % tabs.length;
    else if (e.key === "Home") j = 0;
    else if (e.key === "End") j = tabs.length - 1;
    else return;
    e.preventDefault();
    const next = tabs[j]?.[0];
    if (!next) return;
    ontab(next);
    const btn = e.currentTarget as HTMLElement;
    btn.parentElement?.querySelector<HTMLElement>(`[data-tab="${next}"]`)?.focus();
  }
</script>

<header class="bar">
  <div class="topbar">
    <span class="brand">Wrapped</span>
    <nav class="tabs" aria-label="View">
      {#each tabs as [t, title] (t)}
        <button
          type="button"
          class="tab"
          class:active={tab === t}
          data-tab={t}
          aria-current={tab === t ? "page" : undefined}
          onkeydown={tabKey}
          onclick={() => ontab(t)}>{title}</button
        >
      {/each}
    </nav>
    <button
      id="settings-gear"
      type="button"
      class="icon-btn settings-btn"
      aria-label="Settings"
      aria-haspopup="dialog"
      title="Settings"
      onclick={(e) => onsettings(e.currentTarget)}
    >
      <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
        <path
          d="M6.7 1.6h2.6l.4 1.7c.4.15.8.35 1.1.6l1.6-.6 1.3 2.2-1.3 1.2c.05.25.08.5.08.75s-.03.5-.08.75l1.3 1.2-1.3 2.2-1.6-.6c-.35.25-.7.45-1.1.6l-.4 1.7H6.7l-.4-1.7c-.4-.15-.8-.35-1.1-.6l-1.6.6-1.3-2.2 1.3-1.2A4 4 0 0 1 3.5 8c0-.25.03-.5.08-.75l-1.3-1.2 1.3-2.2 1.6.6c.35-.25.7-.45 1.1-.6z"
          fill="none"
          stroke="currentColor"
          stroke-width="1.2"
          stroke-linejoin="round"
        />
        <circle cx="8" cy="8" r="2" fill="none" stroke="currentColor" stroke-width="1.2" />
      </svg>
    </button>
  </div>

  <div class="subbar">
    {#if tab === "overview"}
      <h1 class="title">{rangeLabel(selection)}</h1>
      <div class="actions">
        <div class="presets" role="group" aria-label="Range">
          {#each presets as [p, title] (p)}
            <button
              type="button"
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
            type="button"
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
              type="button"
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
        <button
          type="button"
          class="icon-btn"
          class:spinning={refreshing}
          aria-label="Refresh"
          title="Refresh"
          onclick={onrefresh}
        >
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
    {:else if tab === "now"}
      <h1 class="title">Now</h1>
      <p class="subtitle muted">Live hardware activity · updated every second</p>
    {:else}
      <h1 class="title">Recaps</h1>
      <p class="subtitle muted">Weekly, monthly, and yearly summaries</p>
    {/if}
  </div>
</header>

<style>
  .bar {
    display: grid;
    gap: 12px;
  }
  .topbar {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    height: 36px;
    border-bottom: 1px solid var(--divider);
  }
  .brand {
    justify-self: start;
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.01em;
    color: var(--text);
  }
  .tabs {
    justify-self: center;
    display: flex;
    align-items: center;
    gap: 18px;
    height: 100%;
  }
  .tab {
    position: relative;
    height: 100%;
    padding: 0 4px;
    background: transparent;
    border: 0;
    border-radius: 4px;
    color: var(--text-dim);
    font-size: 13px;
    font-weight: 500;
    display: inline-flex;
    align-items: center;
  }
  .tab:hover:not(:disabled) {
    color: var(--text);
    border-color: transparent;
  }
  .tab.active {
    color: var(--text);
  }
  .tab.active::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    bottom: -1px;
    height: 1.5px;
    background: var(--accent);
  }
  .tab:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .settings-btn {
    justify-self: end;
  }
  .subbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    min-height: 30px;
    gap: 8px 16px;
  }
  .title {
    margin: 0;
    font-size: 22px;
    font-weight: 600;
    line-height: 1.2;
    letter-spacing: -0.025em;
  }
  .subtitle {
    margin: 0;
    font-size: 12px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .presets {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .presets button {
    background: transparent;
    border-color: transparent;
    border-radius: var(--radius-sm);
    color: var(--text-dim);
    padding: 3px 8px;
    font-size: 12px;
  }
  .presets button:hover:not(:disabled) {
    color: var(--text);
    background: var(--hover-bg);
    border-color: transparent;
  }
  .presets button.active {
    background: rgba(var(--accent-rgb), 0.14);
    border-color: transparent;
    color: var(--text);
  }
  /* The popover is absolutely positioned: opening it moves nothing. */
  .dates {
    position: relative;
  }
  .range-btn {
    font-size: 12px;
    padding: 3px 8px;
    background: transparent;
    white-space: nowrap;
  }
  .range-btn.open {
    border-color: rgba(var(--accent-rgb), 0.45);
  }
  .asof {
    font-size: 12px;
    white-space: nowrap;
    border-radius: 5px;
    padding: 2px 5px;
    transition: background-color 0.9s ease, color 0.9s ease;
  }
  .asof.flash {
    background: rgba(var(--accent-rgb), 0.22);
    color: var(--text);
    transition: none;
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    padding: 0;
    background: transparent;
    border-color: transparent;
    color: var(--text-dim);
  }
  .icon-btn:hover:not(:disabled) {
    color: var(--text);
    background: var(--hover-bg);
    border-color: transparent;
  }
  .icon-btn.spinning svg {
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
