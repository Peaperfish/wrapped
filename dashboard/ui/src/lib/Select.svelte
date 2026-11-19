<script lang="ts">
  // The one dropdown of the app (replaces the native <select>): a field-style
  // button and a list in the dark theme. Arrows / Home / End move, Enter picks,
  // Esc closes, typing jumps to a matching item (or filters, with `searchable`).
  // The list is drawn outside the card (on <body>) so it is never clipped, opens
  // upward when there is no room below, and stays inside the window.
  import { tick } from "svelte";

  export interface Option {
    value: string;
    label: string;
    /** Shown in full in a tooltip (e.g. the bundle id). */
    title?: string;
  }

  interface Props {
    options: Option[];
    value: string;
    placeholder?: string;
    disabled?: boolean;
    searchable?: boolean;
    ariaLabel?: string;
    onchange?: (value: string) => void;
  }
  let {
    options,
    value = $bindable(),
    placeholder = "Choose…",
    disabled = false,
    searchable = false,
    ariaLabel,
    onchange,
  }: Props = $props();

  const uid = `sel-${Math.random().toString(36).slice(2, 8)}`;
  const ROW = 28;
  const MAX_H = 280;
  const GAP = 4;
  const MARGIN = 8;

  let open = $state(false);
  let trigger = $state<HTMLButtonElement>();
  let popup = $state<HTMLDivElement>();
  let list = $state<HTMLDivElement>();
  let query = $state("");
  let hi = $state(0);
  let pos = $state({ left: 0, width: 0, top: 0, bottom: 0, up: false, maxHeight: MAX_H });
  let typed = "";
  let typedAt = 0;

  const shown = $derived(
    searchable && query.trim()
      ? options.filter((o) => o.label.toLowerCase().includes(query.trim().toLowerCase()))
      : options,
  );
  const current = $derived(options.find((o) => o.value === value));

  function place() {
    if (!trigger) return;
    const r = trigger.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const width = Math.min(Math.max(r.width, 200), vw - 2 * MARGIN);
    const left = Math.max(MARGIN, Math.min(r.left, vw - width - MARGIN));
    const below = vh - r.bottom - GAP - MARGIN;
    const above = r.top - GAP - MARGIN;
    const want = Math.min(MAX_H, shown.length * ROW + 10 + (searchable ? 44 : 0));
    const up = below < want && above > below;
    const room = up ? above : below;
    pos = {
      left,
      width,
      top: r.bottom + GAP,
      bottom: vh - r.top + GAP,
      up,
      maxHeight: Math.max(80, Math.min(MAX_H, room)),
    };
  }

  async function show() {
    if (disabled || open) return;
    query = "";
    const i = options.findIndex((o) => o.value === value);
    hi = Math.max(i, 0);
    place();
    open = true;
    await tick();
    place();
    (searchable ? popup?.querySelector("input") : list)?.focus({ preventScroll: true });
    reveal();
  }

  function hide(refocus = true) {
    if (!open) return;
    open = false;
    if (refocus) trigger?.focus({ preventScroll: true });
  }

  function pick(o: Option | undefined) {
    if (!o) return;
    value = o.value;
    onchange?.(o.value);
    hide();
  }

  function reveal() {
    void tick().then(() =>
      list?.querySelector<HTMLElement>(`[data-i="${hi}"]`)?.scrollIntoView({ block: "nearest" }),
    );
  }

  function move(to: number) {
    if (shown.length === 0) return;
    hi = Math.max(0, Math.min(shown.length - 1, to));
    reveal();
  }

  function onTriggerKey(e: KeyboardEvent) {
    if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
      e.preventDefault();
      void show();
    }
  }

  function onPopupKey(e: KeyboardEvent) {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        return move(hi + 1);
      case "ArrowUp":
        e.preventDefault();
        return move(hi - 1);
      case "Home":
        e.preventDefault();
        return move(0);
      case "End":
        e.preventDefault();
        return move(shown.length - 1);
      case "PageDown":
        e.preventDefault();
        return move(hi + 8);
      case "PageUp":
        e.preventDefault();
        return move(hi - 8);
      case "Enter":
        e.preventDefault();
        return pick(shown[hi]);
      case "Escape":
        // Only the list closes, not the dialog behind it.
        e.preventDefault();
        e.stopPropagation();
        return hide();
      case "Tab":
        return hide(false);
    }
    if (!searchable && e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      const now = Date.now();
      typed = now - typedAt > 700 ? e.key : typed + e.key;
      typedAt = now;
      const t = typed.toLowerCase();
      const from = typed.length === 1 ? hi + 1 : hi;
      const n = shown.length;
      for (let k = 0; k < n; k++) {
        const i = (from + k) % n;
        if (shown[i]!.label.toLowerCase().startsWith(t)) {
          e.preventDefault();
          return move(i);
        }
      }
    }
  }

  $effect(() => {
    if (!open) return;
    const down = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!popup?.contains(t) && !trigger?.contains(t)) hide(false);
    };
    const away = () => hide(false);
    window.addEventListener("pointerdown", down, true);
    window.addEventListener("resize", away);
    window.addEventListener("blur", away);
    return () => {
      window.removeEventListener("pointerdown", down, true);
      window.removeEventListener("resize", away);
      window.removeEventListener("blur", away);
    };
  });

  // Typing in the search box restarts the highlight at the first match.
  $effect(() => {
    void query;
    hi = 0;
  });

  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }
</script>

<button
  bind:this={trigger}
  type="button"
  class="sel"
  class:open
  {disabled}
  role="combobox"
  aria-haspopup="listbox"
  aria-expanded={open}
  aria-controls={uid}
  aria-label={ariaLabel}
  onclick={() => (open ? hide() : void show())}
  onkeydown={onTriggerKey}
>
  <span class="text" class:placeholder={!current}>{current?.label ?? placeholder}</span>
  <svg viewBox="0 0 10 6" width="10" height="6" aria-hidden="true">
    <path d="M1 1l4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
  </svg>
</button>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    use:portal
    bind:this={popup}
    class="menu"
    style:left="{pos.left}px"
    style:width="{pos.width}px"
    style:top={pos.up ? "auto" : `${pos.top}px`}
    style:bottom={pos.up ? `${pos.bottom}px` : "auto"}
    style:max-height="{pos.maxHeight}px"
    onkeydown={onPopupKey}
  >
    {#if searchable}
      <input
        class="search"
        type="text"
        placeholder="Search…"
        aria-label="Search"
        autocomplete="off"
        spellcheck="false"
        bind:value={query}
      />
    {/if}
    <div
      bind:this={list}
      class="list"
      id={uid}
      role="listbox"
      tabindex="-1"
      aria-activedescendant="{uid}-{hi}"
    >
      {#each shown as o, i (o.value)}
        <div
          id="{uid}-{i}"
          data-i={i}
          class="opt"
          class:hi={i === hi}
          class:chosen={o.value === value}
          role="option"
          aria-selected={o.value === value}
          title={o.title ?? o.label}
          tabindex="-1"
          onpointermove={() => (hi = i)}
          onclick={() => pick(o)}
          onkeydown={() => {}}
        >
          {o.label}
        </div>
      {:else}
        <div class="none faint">No matches</div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .sel {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-width: 0;
    background: var(--field-bg);
    color: var(--text);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 5px 8px;
    text-align: left;
  }
  .sel.open {
    border-color: rgba(var(--accent-rgb), 0.5);
  }
  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .placeholder {
    color: var(--text-dim);
  }
  svg {
    flex: none;
    color: var(--text-dim);
  }
  .menu {
    position: fixed;
    z-index: 100;
    display: flex;
    flex-direction: column;
    background: var(--menu-bg);
    color: var(--text);
    border: 1px solid var(--modal-border);
    border-radius: var(--radius-sm);
    box-shadow: var(--modal-shadow);
    backdrop-filter: blur(var(--overlay-blur));
    -webkit-backdrop-filter: blur(var(--overlay-blur));
    font-size: 13px;
    overflow: hidden;
  }
  .search {
    flex: none;
    margin: 6px 6px 2px;
    background: var(--field-bg);
    color: var(--text);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 4px 8px;
    user-select: text;
    -webkit-user-select: text;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 4px;
    outline: none;
  }
  .opt {
    height: 28px;
    line-height: 26px;
    padding: 0 8px;
    border-radius: 5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .opt.hi {
    background: var(--hover-bg);
  }
  .opt.chosen {
    color: var(--accent);
  }
  .none {
    padding: 6px 8px;
  }
</style>
