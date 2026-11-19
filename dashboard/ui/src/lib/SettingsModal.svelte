<script lang="ts">
  import { lockScroll } from "./scrollLock";
  // Settings: a centred glass dialog over the dimmed, blurred dashboard.
  // Section list on the left, the section on the right. Closes with Escape,
  // the ✕ button or a click outside; focus stays inside while open and goes
  // back to the element that opened it (the gear button) on close.
  //
  // Every change is a request to the collector (the only writer); the dialog
  // never edits the database.
  import { onMount, tick } from "svelte";
  import AppearanceSection from "./settings/AppearanceSection.svelte";
  import AiSourcesSection from "./settings/AiSourcesSection.svelte";
  import DataSection from "./settings/DataSection.svelte";
  import ExcludeSection from "./settings/ExcludeSection.svelte";
  import FoldersSection from "./settings/FoldersSection.svelte";
  import TrackingSection from "./settings/TrackingSection.svelte";
  import type { Meta } from "./types";

  interface Props {
    meta: Meta;
    /** Fetch the settings again after a change. */
    onchanged: () => Promise<void>;
    /** Close; focus goes back to the opener. */
    onclose: () => void;
  }
  let { meta, onchanged, onclose }: Props = $props();

  type SectionId = "tracking" | "exclude" | "folders" | "ai" | "appearance" | "data";
  const sections = $derived<{ id: SectionId; title: string }[]>([
    { id: "tracking", title: "Tracking" },
    { id: "exclude", title: "Exclude list" },
    { id: "folders", title: "Folders" },
    ...(meta.collectors.some((c) => c.name === "ai_usage")
      ? [{ id: "ai" as const, title: "AI sources" }]
      : []),
    { id: "appearance", title: "Appearance" },
    { id: "data", title: "Data" },
  ]);

  let current = $state<SectionId>("tracking");
  let dialog: HTMLDivElement | undefined = $state();

  const FOCUSABLE =
    'button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

  function focusables(): HTMLElement[] {
    return dialog ? [...dialog.querySelectorAll<HTMLElement>(FOCUSABLE)] : [];
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onclose();
      return;
    }
    if (e.key !== "Tab") return;
    const f = focusables();
    if (f.length === 0) {
      e.preventDefault();
      return;
    }
    const active = document.activeElement;
    const outside = !dialog?.contains(active);
    if (e.shiftKey && (active === f[0] || outside)) {
      e.preventDefault();
      f.at(-1)?.focus();
    } else if (!e.shiftKey && (active === f.at(-1) || outside)) {
      e.preventDefault();
      f[0]?.focus();
    }
  }

  function navKey(e: KeyboardEvent) {
    const i = sections.findIndex((s) => s.id === current);
    let j = i;
    if (e.key === "ArrowDown") j = (i + 1) % sections.length;
    else if (e.key === "ArrowUp") j = (i - 1 + sections.length) % sections.length;
    else return;
    e.preventDefault();
    current = sections[j]?.id ?? current;
    void tick().then(() => dialog?.querySelector<HTMLElement>(`#tab-${current}`)?.focus());
  }

  onMount(() => {
    // The selected tab is the first stop.
    dialog?.querySelector<HTMLElement>(`#tab-${current}`)?.focus();
    // The page behind must not scroll while the dialog is open.
    return lockScroll();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="overlay"
  onmousedown={(e) => {
    if (e.target === e.currentTarget) onclose();
  }}
  {onkeydown}
>
  <div
    class="modal"
    role="dialog"
    aria-modal="true"
    aria-labelledby="settings-title"
    bind:this={dialog}
  >
    <nav aria-label="Settings sections">
      <h2 id="settings-title">Settings</h2>
      <div class="tabs" role="tablist" aria-orientation="vertical" tabindex="-1" onkeydown={navKey}>
        {#each sections as s (s.id)}
          <button
            id="tab-{s.id}"
            role="tab"
            aria-selected={current === s.id}
            aria-controls="panel-{s.id}"
            tabindex={current === s.id ? 0 : -1}
            class:active={current === s.id}
            onclick={() => (current = s.id)}>{s.title}</button
          >
        {/each}
      </div>
    </nav>

    <div class="content">
      <button class="close" aria-label="Close settings" title="Close (Esc)" onclick={onclose}>✕</button>
      <div class="panel" role="tabpanel" id="panel-{current}" aria-labelledby="tab-{current}">
        {#if current === "tracking"}
          <TrackingSection {meta} {onchanged} />
        {:else if current === "exclude"}
          <ExcludeSection {meta} {onchanged} />
        {:else if current === "folders"}
          <FoldersSection {meta} {onchanged} />
        {:else if current === "ai"}
          <AiSourcesSection {meta} {onchanged} />
        {:else if current === "appearance"}
          <AppearanceSection {meta} {onchanged} />
        {:else if current === "data"}
          <DataSection {meta} {onchanged} />
        {/if}
      </div>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: var(--overlay-bg);
    backdrop-filter: blur(var(--overlay-blur));
    -webkit-backdrop-filter: blur(var(--overlay-blur));
    padding: 24px;
    overscroll-behavior: contain;
  }
  .modal {
    width: min(880px, 100%);
    height: min(640px, 100%);
    display: grid;
    grid-template-columns: 190px 1fr;
    background: var(--modal-bg);
    border: 1px solid var(--modal-border);
    border-radius: var(--radius);
    box-shadow: var(--modal-shadow);
    backdrop-filter: blur(var(--card-blur)) saturate(140%);
    -webkit-backdrop-filter: blur(var(--card-blur)) saturate(140%);
    overflow: hidden;
  }
  nav {
    border-right: 1px solid var(--divider);
    padding: 18px 10px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  nav h2 {
    margin: 0 8px;
    font-size: 13px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .tabs {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tabs button {
    text-align: left;
    background: transparent;
    border-color: transparent;
    color: var(--text-dim);
    padding: 7px 10px;
  }
  .tabs button:hover {
    background: var(--hover-bg);
  }
  .tabs button.active {
    background: rgba(var(--accent-rgb), 0.16);
    border-color: rgba(var(--accent-rgb), 0.35);
    color: var(--text);
  }
  .content {
    position: relative;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .close {
    position: absolute;
    top: 10px;
    right: 10px;
    width: 28px;
    height: 28px;
    padding: 0;
    background: transparent;
    border-color: transparent;
    color: var(--text-dim);
    z-index: 1;
  }
  .close:hover:not(:disabled) {
    background: var(--hover-bg);
  }
  .panel {
    overflow: auto;
    overscroll-behavior: contain;
    padding: 20px 24px 24px;
    flex: 1;
    user-select: text;
    -webkit-user-select: text;
  }
</style>
