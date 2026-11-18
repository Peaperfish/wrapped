<script lang="ts">
  // A popover with two months side by side (Monday-first) for picking a date
  // range. Nothing is applied until the end day is chosen (second click,
  // Return); Escape or a click outside just closes it.
  //
  // Mouse: click a start day, then an end day (swapped if before the start);
  // hover previews the band. Keyboard: arrows move the focused day, shift+arrows
  // extend the range from where the focus was, Space picks like a click,
  // Return applies. Days after today cannot be chosen.
  import { onMount, untrack } from "svelte";
  import {
    type Range,
    type Ymd,
    addDays,
    dayNumber,
    fromDayNumber,
    monthLong,
    ordered,
    rangeLabel,
    weekday,
    weekdayShort,
    yearOf,
  } from "./dates";

  interface Props {
    selection: Range;
    today: Ymd;
    onapply: (r: Range) => void;
    onclose: () => void;
    /** Elements whose own handlers decide about opening (the trigger button). */
    trigger?: HTMLElement;
  }
  let { selection, today, onapply, onclose, trigger }: Props = $props();

  // A month as year * 12 + zero-based month.
  const monthOf = (d: Ymd): number => yearOf(d) * 12 + Number(d.slice(5, 7)) - 1;
  const first = (mi: number): Ymd =>
    `${Math.floor(mi / 12)}-${String((mi % 12) + 1).padStart(2, "0")}-01`;
  const lastMonth = $derived(monthOf(today));

  // Initial focus: the end of the current selection (never after today).
  // (Read once on open: the popover works on a draft.)
  const start = untrack(() => (selection.to > today ? today : selection.to));
  let cur = $state<Ymd>(start);
  let anchor = $state<Ymd | null>(null);
  let touched = $state(false);
  let kbd = $state(false);
  // Left month: the right one shows the selection's end month.
  let left = $state(untrack(() => Math.min(monthOf(start), monthOf(today)) - 1));

  const draft: Range = $derived(
    anchor ? ordered(anchor, cur) : touched ? { from: cur, to: cur } : selection,
  );

  interface Day {
    date: Ymd;
    n: number;
    future: boolean;
  }
  interface Month {
    mi: number;
    title: string;
    blanks: number;
    days: Day[];
  }
  function month(mi: number): Month {
    const f = first(mi);
    const next = first(mi + 1);
    const count = dayNumber(next) - dayNumber(f);
    return {
      mi,
      title: `${monthLong(mi % 12)} ${Math.floor(mi / 12)}`,
      blanks: weekday(f),
      days: Array.from({ length: count }, (_, i) => {
        const date = fromDayNumber(dayNumber(f) + i);
        return { date, n: i + 1, future: date > today };
      }),
    };
  }
  const months = $derived([month(left), month(left + 1)]);

  function reveal(d: Ymd) {
    const m = monthOf(d);
    if (m < left) left = m;
    else if (m > left + 1) left = m - 1;
  }

  function pick(d: Ymd) {
    if (d > today) return;
    cur = d;
    touched = true;
    if (anchor) onapply(ordered(anchor, d));
    else anchor = d;
  }

  function move(next: Ymd, extend: boolean) {
    if (next > today) next = today;
    if (extend) anchor ??= cur;
    else anchor = null;
    touched = true;
    cur = next;
    reveal(next);
  }

  let root: HTMLDivElement | undefined = $state();

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onclose();
      return;
    }
    kbd = true;
    const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 }[e.key];
    if (step !== undefined) {
      e.preventDefault();
      move(addDays(cur, step), e.shiftKey);
    } else if (e.key === "Enter") {
      e.preventDefault();
      onapply(draft);
    } else if (e.key === " " && !(e.target instanceof HTMLButtonElement && e.target.dataset.nav)) {
      e.preventDefault();
      pick(cur);
    }
  }

  onMount(() => {
    root?.focus({ preventScroll: true });
    const down = (e: PointerEvent) => {
      const t = e.target as Node;
      if (root?.contains(t) || trigger?.contains(t)) return;
      onclose();
    };
    document.addEventListener("pointerdown", down, true);
    return () => document.removeEventListener("pointerdown", down, true);
  });

  const inBand = (d: Ymd): boolean => draft.from <= d && d <= draft.to;
</script>

<div
  class="pop"
  role="dialog"
  aria-label="Choose a date range"
  tabindex="-1"
  bind:this={root}
  {onkeydown}
  onpointermove={() => (kbd = false)}
>
  <div class="nav">
    <button class="arrow" data-nav="1" aria-label="Previous month" onclick={() => (left -= 1)}>‹</button>
    <button
      class="arrow"
      data-nav="1"
      aria-label="Next month"
      disabled={left + 1 >= lastMonth}
      onclick={() => (left += 1)}>›</button
    >
  </div>

  <div class="months">
    {#each months as m (m.mi)}
      <div class="month">
        <div class="title">{m.title}</div>
        <div class="grid" role="grid" aria-label={m.title}>
          {#each [0, 1, 2, 3, 4, 5, 6] as w (w)}
            <span class="wd faint">{weekdayShort(w).slice(0, 2)}</span>
          {/each}
          {#each { length: m.blanks } as _, i (i)}
            <span></span>
          {/each}
          {#each m.days as d (d.date)}
            {@const inb = inBand(d.date)}
            {@const wd = (m.blanks + d.n - 1) % 7}
            <button
              class="day"
              class:band={inb}
              class:cap={d.date === draft.from || d.date === draft.to}
              class:capl={d.date === draft.from || wd === 0 || d.n === 1}
              class:capr={d.date === draft.to || wd === 6 || d.n === m.days.length}
              class:solid-l={d.date === draft.from}
              class:solid-r={d.date === draft.to}
              class:today={d.date === today}
              class:future={d.future}
              class:focus={kbd && d.date === cur}
              tabindex="-1"
              disabled={d.future}
              aria-label={d.date}
              aria-pressed={inb}
              onclick={() => pick(d.date)}
              onpointerenter={() => {
                if (anchor && !d.future) cur = d.date;
              }}><span>{d.n}</span></button
            >
          {/each}
        </div>
      </div>
    {/each}
  </div>

  <div class="foot">
    <span class="muted num">{rangeLabel(draft)}</span>
    <button class="link" onclick={() => (left = lastMonth - 1)}>Today</button>
  </div>
</div>

<style>
  .pop {
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    z-index: 30;
    padding: 12px 14px 10px;
    border-radius: var(--radius);
    background: rgba(18, 21, 30, 0.88);
    border: 1px solid var(--card-border);
    box-shadow: var(--card-shadow);
    backdrop-filter: blur(var(--card-blur));
    -webkit-backdrop-filter: blur(var(--card-blur));
    outline: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .nav {
    position: absolute;
    top: 10px;
    left: 12px;
    right: 12px;
    display: flex;
    justify-content: space-between;
    pointer-events: none;
  }
  .arrow {
    pointer-events: auto;
    padding: 0 9px;
    line-height: 22px;
    background: transparent;
    border-color: transparent;
    color: var(--text-dim);
  }
  .arrow:hover:not(:disabled) {
    background: var(--card-bg-strong);
    color: var(--text);
  }
  .months {
    display: flex;
    gap: 22px;
  }
  .title {
    text-align: center;
    font-weight: 600;
    font-size: 13px;
    height: 24px;
    line-height: 24px;
    margin-bottom: 4px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(7, 32px);
    grid-auto-rows: 30px;
    align-items: center;
  }
  .wd {
    text-align: center;
    font-size: 11px;
  }
  .day {
    position: relative;
    width: 32px;
    height: 30px;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--text);
    font-family: var(--mono);
    font-size: 12px;
    display: grid;
    place-items: center;
  }
  .day span {
    position: relative;
    z-index: 1;
    width: 28px;
    height: 26px;
    line-height: 26px;
    border-radius: 14px;
  }
  .day:hover:not(:disabled):not(.cap) span {
    background: var(--card-bg-strong);
  }
  .day.band {
    background: rgba(var(--accent-rgb), 0.18);
  }
  .day.capl {
    border-top-left-radius: 14px;
    border-bottom-left-radius: 14px;
  }
  .day.capr {
    border-top-right-radius: 14px;
    border-bottom-right-radius: 14px;
  }
  /* Solid start / end caps. */
  .day.cap span {
    background: var(--accent);
    color: var(--accent-ink);
    font-weight: 700;
  }
  .day.today span {
    box-shadow: inset 0 0 0 1.5px rgba(var(--accent-rgb), 0.9);
  }
  .day.today.cap span {
    box-shadow: inset 0 0 0 1.5px var(--accent-ink);
  }
  .day.focus span {
    outline: 1.5px solid var(--text);
    outline-offset: 1px;
  }
  .day.future {
    color: var(--text-faint);
    opacity: 0.45;
    cursor: not-allowed;
  }
  .foot {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 12px;
    padding-top: 4px;
    border-top: 1px solid var(--divider);
  }
  .link {
    background: none;
    border: 0;
    padding: 2px 4px;
    color: var(--accent);
    font-size: 12px;
  }
  .link:hover {
    text-decoration: underline;
  }
</style>
