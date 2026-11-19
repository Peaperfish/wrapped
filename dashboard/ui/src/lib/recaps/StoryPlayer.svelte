<script lang="ts">
  // The story player: a full-window overlay with a 9:16 stage. Slides
  // auto-advance every 6 s; tap the right / left half (or ← →) for next /
  // previous, Space to pause, press and hold to pause, Esc or × to close.
  // Each slide is one SVG (slides.ts); the same SVG, without animations, is
  // what "Save slide" / "Save all" turn into 1080 × 1920 PNGs.
  import { onDestroy, onMount, tick } from "svelte";
  import { errorText, getAppNames, getRecap, revealExport, saveRecapPng } from "../api";
  import { lockScroll } from "../scrollLock";
  import type { RecapJson, SlideJson } from "../types";
  import { type Fmt, type SlideCtx, currentLook, fmt, slideFile, slideSvg, svgToPng } from "./slides";

  interface Props {
    id: string;
    onclose: () => void;
  }
  let { id, onclose }: Props = $props();

  const DURATION = 6000;
  const HOLD_MS = 250;
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");

  let recap = $state.raw<RecapJson | null>(null);
  let error = $state<string | null>(null);
  let names = $state.raw<Record<string, string>>({});
  let look = $state.raw(currentLook());
  let index = $state(0);
  let paused = $state(false);
  let holding = $state(false);
  let busy = $state(false);
  let hidden = $state(document.visibilityState !== "visible");
  let toast = $state.raw<{ text: string; path: string | null; error?: boolean } | null>(null);

  let overlay: HTMLDivElement | undefined = $state();
  let stage: HTMLDivElement | undefined = $state();
  const opener = document.activeElement as HTMLElement | null;
  const unlock = lockScroll();

  const slides = $derived(recap?.slides ?? []);
  const slide = $derived<SlideJson | undefined>(slides[index]);
  const ctx = $derived<SlideCtx | null>(recap ? { recap, names, theme: look.theme, accent: look.accent } : null);
  const svg = $derived(slide && ctx ? slideSvg(slide, ctx, true) : "");
  const stopped = $derived(paused || holding || busy || hidden || index >= slides.length - 1);
  const periodDir = $derived(id.replace(":", "-"));

  onMount(async () => {
    overlay?.focus();
    try {
      const r = await getRecap(id);
      const apps = new Set<string>();
      for (const s of r.slides) {
        if (s.kind === "top_apps") s.apps.forEach((a) => apps.add(a.key));
        if (s.kind === "records" && s.longest_session) apps.add(s.longest_session.app);
        if (s.kind === "summary") s.items.forEach((h) => h.id === "top_app" && h.text && apps.add(h.text));
      }
      if (apps.size > 0) {
        try {
          names = await getAppNames([...apps]);
        } catch {
          /* ids shown as they are */
        }
      }
      look = currentLook();
      recap = r;
    } catch (e) {
      error = errorText(e);
    }
  });

  // ---------- auto-advance: a timer that keeps its remaining time across pauses ----------
  let timer: ReturnType<typeof setTimeout> | undefined;
  let remaining = DURATION;
  let armedAt = 0;
  let shownIndex = -1;

  function disarm() {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
      remaining = Math.max(0, remaining - (performance.now() - armedAt));
    }
  }
  $effect(() => {
    const i = index;
    const stop = stopped;
    if (!recap) return;
    if (i !== shownIndex) {
      shownIndex = i;
      disarm();
      remaining = DURATION;
    }
    if (stop) {
      disarm();
    } else if (timer === undefined) {
      armedAt = performance.now();
      timer = setTimeout(() => {
        timer = undefined;
        next();
      }, remaining);
    }
  });

  // ---------- count-up after each slide appears ----------
  let raf = 0;
  $effect(() => {
    void svg;
    cancelAnimationFrame(raf);
    if (!stage || !svg) return;
    if (reduced.matches) return;
    const els = [...stage.querySelectorAll<SVGTextElement>("[data-n]")].map((el) => ({
      el,
      n: Number(el.dataset.n),
      f: el.dataset.f as Fmt,
      done: el.textContent ?? "",
    }));
    if (els.length === 0) return;
    const t0 = performance.now();
    const dur = 1100;
    const frame = (t: number) => {
      const k = Math.min(1, (t - t0) / dur);
      const e = 1 - (1 - k) ** 3;
      for (const x of els) x.el.textContent = k >= 1 ? x.done : fmt(x.f, x.n * e);
      if (k < 1) raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  });

  function next() {
    if (index < slides.length - 1) index++;
  }
  function prev() {
    if (index > 0) index--;
  }

  async function close() {
    disarm();
    onclose();
    await tick();
    opener?.focus?.();
  }

  // ---------- input ----------
  let downAt = 0;
  let downX = 0;
  function pointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    downAt = performance.now();
    downX = e.clientX;
    holding = true;
  }
  function pointerup(e: PointerEvent) {
    if (!holding) return;
    holding = false;
    if (performance.now() - downAt > HOLD_MS) return; // a hold: just resume
    const r = (stage ?? overlay)?.getBoundingClientRect();
    const mid = r ? r.left + r.width / 2 : window.innerWidth / 2;
    if ((e.clientX || downX) < mid) prev();
    else next();
  }
  function keydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      void close();
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      next();
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      prev();
    } else if (e.key === " ") {
      e.preventDefault();
      paused = !paused;
    }
  }
  const stop = (e: Event) => e.stopPropagation();

  $effect(() => {
    const f = () => (hidden = document.visibilityState !== "visible");
    document.addEventListener("visibilitychange", f);
    return () => document.removeEventListener("visibilitychange", f);
  });

  // ---------- export ----------
  const isWindows = navigator.userAgent.includes("Windows");
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  function showToast(t: typeof toast) {
    toast = t;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 8000);
  }
  const shortPath = (p: string) => {
    const i = p.search(/[\\/]Downloads[\\/]/);
    const dir = p.replace(/[\\/][^\\/]*$/, "");
    return i >= 0 ? "~" + dir.slice(i) : dir;
  };

  async function save(which: "one" | "all") {
    if (!recap || !ctx || busy) return;
    busy = true;
    const list = which === "one" ? [index] : slides.map((_, i) => i);
    const staticCtx = { ...ctx };
    let first: string | null = null;
    try {
      for (const i of list) {
        const s = slides[i];
        if (!s) continue;
        const png = await svgToPng(slideSvg(s, staticCtx, false));
        const path = await saveRecapPng(periodDir, slideFile(i, s), png);
        first ??= path;
      }
      if (first) {
        const n = list.length;
        showToast({ text: `Saved ${n === 1 ? "1 slide" : `${n} slides`} to ${shortPath(first)}`, path: first });
      }
    } catch (e) {
      showToast({ text: `Could not save: ${errorText(e)}`, path: null, error: true });
    } finally {
      busy = false;
    }
  }

  onDestroy(() => {
    clearTimeout(timer);
    clearTimeout(toastTimer);
    cancelAnimationFrame(raf);
    unlock();
  });
</script>

<svelte:window onkeydown={keydown} />

<div
  class="overlay"
  role="dialog"
  aria-modal="true"
  aria-label={recap ? `Recap ${recap.label}` : "Recap"}
  tabindex="-1"
  bind:this={overlay}
  onpointerdown={pointerdown}
  onpointerup={pointerup}
  onpointercancel={() => (holding = false)}
>
  <div class="toolbar" role="toolbar" tabindex="-1" onpointerdown={stop} onpointerup={stop}>
    {#if recap}
      <button onclick={() => (paused = !paused)} aria-pressed={paused} title="Pause (Space)">
        {paused ? "Play" : "Pause"}
      </button>
      <button onclick={() => save("one")} disabled={busy}>Save slide</button>
      <button onclick={() => save("all")} disabled={busy}>{busy ? "Saving…" : "Save all"}</button>
    {/if}
    <button class="close" aria-label="Close (Esc)" title="Close (Esc)" onclick={close}>
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
        <path d="M3.5 3.5l9 9M12.5 3.5l-9 9" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
      </svg>
    </button>
  </div>

  <div class="stage" bind:this={stage} class:paused={stopped}>
    {#if error}
      <div class="message"><p class="error">{error}</p></div>
    {:else if !recap}
      <div class="message"><p class="muted">Loading…</p></div>
    {:else}
      {#key index}
        <div class="slide" aria-live="polite">
          <!-- Our own SVG string built from numbers and escaped names (slides.ts). -->
          <!-- eslint-disable-next-line svelte/no-at-html-tags -->
          {@html svg}
        </div>
      {/key}
      <div class="progress" aria-hidden="true">
        {#each slides as s, i (i)}
          <span class="seg" title={s.kind}>
            {#if i < index}
              <span class="fill full"></span>
            {:else if i === index}
              {#key index}
                <span class="fill run" style:animation-duration="{DURATION}ms"></span>
              {/key}
            {/if}
          </span>
        {/each}
      </div>
      {#if paused}
        <span class="paused-badge">Paused</span>
      {/if}
    {/if}
  </div>

  {#if toast}
    <div class="toast" class:bad={toast.error} role="status" onpointerdown={stop} onpointerup={stop}>
      <span>{toast.text}</span>
      {#if toast.path}
        <button onclick={() => toast?.path && revealExport(toast.path).catch(() => undefined)}>
          {isWindows ? "Show in Explorer" : "Show in Finder"}
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 12px 16px 16px;
    background: var(--player-bg);
    outline: none;
    touch-action: none;
  }
  .toolbar {
    align-self: stretch;
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    outline: none;
  }
  .toolbar button {
    background: var(--player-btn);
    color: var(--player-ink);
    border-color: var(--player-btn-border);
  }
  .close {
    display: inline-flex;
    align-items: center;
    padding: 6px 9px;
  }
  .stage {
    position: relative;
    height: min(calc(100vh - 76px), calc((100vw - 32px) * 16 / 9));
    aspect-ratio: 9 / 16;
    border-radius: 22px;
    overflow: hidden;
    background: var(--card-bg-strong);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.45);
    contain: layout paint;
    cursor: pointer;
  }
  .slide,
  .slide :global(svg) {
    position: absolute;
    inset: 0;
    display: block;
  }
  .message {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 24px;
    text-align: center;
  }
  .progress {
    position: absolute;
    top: 10px;
    left: 12px;
    right: 12px;
    display: flex;
    gap: 4px;
    pointer-events: none;
  }
  .seg {
    flex: 1;
    height: 3px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.3);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: #fff;
    transform-origin: 0 50%;
  }
  .fill.full {
    transform: none;
  }
  .fill.run {
    transform: scaleX(0);
    animation-name: progress;
    animation-timing-function: linear;
    animation-fill-mode: forwards;
  }
  .paused .fill.run {
    animation-play-state: paused;
  }
  @keyframes progress {
    to {
      transform: scaleX(1);
    }
  }
  .paused-badge {
    position: absolute;
    top: 22px;
    left: 50%;
    transform: translateX(-50%);
    padding: 2px 10px;
    border-radius: 10px;
    background: rgba(0, 0, 0, 0.55);
    color: #fff;
    font-size: 12px;
    pointer-events: none;
  }
  .toast {
    position: absolute;
    bottom: 20px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 12px;
    max-width: calc(100vw - 32px);
    padding: 8px 10px 8px 14px;
    border-radius: 12px;
    background: var(--menu-bg);
    border: 1px solid var(--modal-border);
    color: var(--text);
    box-shadow: var(--modal-shadow);
    font-size: 13px;
  }
  .toast span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .toast.bad span {
    color: var(--danger);
  }
  @media (prefers-reduced-motion: reduce) {
    .fill.run {
      animation: none;
      transform: none;
    }
  }
</style>
