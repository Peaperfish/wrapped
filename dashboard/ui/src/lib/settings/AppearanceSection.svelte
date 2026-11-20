<script lang="ts">
  // Theme (System / Light / Dark), colour palette (Graphite / Warm Paper / Forest)
  // and accent colour. A choice is applied in the dashboard at once; the
  // collector stores theme and accent. If it can't (Wrapped not running) the
  // old look comes back and the usual note says why.
  import { sendCtl } from "../api";
  import CtlNote from "../CtlNote.svelte";
  import {
    ACCENTS,
    PALETTES,
    THEMES,
    applyAppearance,
    applyPalette,
    getPalette,
    type Accent,
    type PaletteChoice,
    type ThemeChoice,
  } from "../theme";
  import type { Meta } from "../types";

  interface Props {
    meta: Meta;
    onchanged: () => Promise<void>;
  }
  let { meta, onchanged }: Props = $props();

  let error = $state<unknown>(null);
  // What is on screen now (ahead of meta while a request is in flight).
  let theme = $state<string | null>(null);
  let accent = $state<string | null>(null);
  let palette = $state<PaletteChoice>(getPalette());
  const shownTheme = $derived(theme ?? meta.settings.theme);
  const shownAccent = $derived(accent ?? meta.settings.accent);

  function choosePalette(value: PaletteChoice) {
    if (palette === value) return;
    palette = value;
    applyPalette(value);
  }

  async function choose(key: "theme" | "accent", value: string) {
    const prev = { theme: shownTheme, accent: shownAccent };
    if (prev[key] === value) return;
    error = null;
    if (key === "theme") theme = value;
    else accent = value;
    applyAppearance(key === "theme" ? value : prev.theme, key === "accent" ? value : prev.accent, palette);
    try {
      await sendCtl("set_settings", { settings: { [`appearance.${key}`]: value } });
      await onchanged();
    } catch (e) {
      error = e;
      applyAppearance(meta.settings.theme, meta.settings.accent, palette);
    } finally {
      theme = null;
      accent = null;
    }
  }

  /** Arrow keys move the choice (and focus) within a radio group. */
  function arrows(e: KeyboardEvent, ids: readonly string[], current: string, onPick: (id: string) => void) {
    const d = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    const next = ids[(ids.indexOf(current) + d + ids.length) % ids.length];
    if (!next) return;
    onPick(next);
    const group = e.currentTarget as HTMLElement;
    group.querySelector<HTMLElement>(`[data-id="${next}"]`)?.focus();
  }

  const themeIds = THEMES.map((t) => t.id);
  const paletteIds = PALETTES.map((p) => p.id);
  const cap = (s: string) => s[0]?.toUpperCase() + s.slice(1);
</script>

<h3>Appearance</h3>
<p class="muted lead">How the dashboard looks. System follows your Mac's Light / Dark setting.</p>

<CtlNote {error} />

<section>
  <h4 id="theme-label">Theme</h4>
  <div
    class="segmented"
    role="radiogroup"
    aria-labelledby="theme-label"
    tabindex="-1"
    onkeydown={(e) => arrows(e, themeIds, shownTheme, (next) => void choose("theme", next))}
  >
    {#each THEMES as t (t.id)}
      <button
        role="radio"
        aria-checked={shownTheme === t.id}
        tabindex={shownTheme === t.id ? 0 : -1}
        data-id={t.id}
        class:on={shownTheme === t.id}
        onclick={() => choose("theme", t.id as ThemeChoice)}>{t.title}</button
      >
    {/each}
  </div>
</section>

<section>
  <h4 id="palette-label">Palette</h4>
  <div
    class="segmented"
    role="radiogroup"
    aria-labelledby="palette-label"
    tabindex="-1"
    onkeydown={(e) => arrows(e, paletteIds, palette, (next) => choosePalette(next as PaletteChoice))}
  >
    {#each PALETTES as p (p.id)}
      <button
        role="radio"
        aria-checked={palette === p.id}
        tabindex={palette === p.id ? 0 : -1}
        data-id={p.id}
        class:on={palette === p.id}
        onclick={() => choosePalette(p.id)}>{p.title}</button
      >
    {/each}
  </div>
</section>

<section>
  <h4 id="accent-label">Accent</h4>
  <div
    class="swatches"
    role="radiogroup"
    aria-labelledby="accent-label"
    tabindex="-1"
    onkeydown={(e) => arrows(e, ACCENTS, shownAccent, (next) => void choose("accent", next))}
  >
    {#each ACCENTS as a (a)}
      <button
        role="radio"
        aria-checked={shownAccent === a}
        aria-label={cap(a)}
        title={cap(a)}
        tabindex={shownAccent === a ? 0 : -1}
        data-id={a}
        data-swatch={a}
        class="swatch"
        onclick={() => choose("accent", a as Accent)}
      >
        {#if shownAccent === a}
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path d="M3.5 8.5l3 3 6-7" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        {/if}
      </button>
    {/each}
  </div>
</section>

<style>
  h3 {
    margin: 0 0 14px;
    font-size: 17px;
  }
  .lead {
    margin: 0 0 14px;
  }
  section {
    border-top: 1px solid var(--divider);
    padding: 14px 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h4 {
    margin: 0;
    font-size: 14px;
  }
  .segmented {
    display: inline-flex;
    align-self: flex-start;
    gap: 2px;
  }
  .segmented button {
    background: transparent;
    border-color: transparent;
    border-radius: var(--radius-sm);
    color: var(--text-dim);
    padding: 4px 10px;
    min-width: 72px;
  }
  .segmented button:hover:not(:disabled) {
    color: var(--text);
    background: var(--hover-bg);
    border-color: transparent;
  }
  .segmented button.on {
    background: rgba(var(--accent-rgb), 0.14);
    border-color: transparent;
    color: var(--text);
  }
  .swatches {
    display: flex;
    gap: 10px;
  }
  .swatch {
    width: 30px;
    height: 30px;
    padding: 0;
    border-radius: 50%;
    border: 2px solid var(--card-border);
    display: grid;
    place-items: center;
    color: var(--accent-ink);
    /* The dark-theme shade on dark, the light-theme shade on light: the swatch
       shows the colour the accent will actually have. */
  }
  .swatch[data-swatch="green"] {
    background: var(--sw-green);
  }
  .swatch[data-swatch="blue"] {
    background: var(--sw-blue);
  }
  .swatch[data-swatch="purple"] {
    background: var(--sw-purple);
  }
  .swatch[data-swatch="orange"] {
    background: var(--sw-orange);
  }
  .swatch[data-swatch="pink"] {
    background: var(--sw-pink);
  }
  .swatch[aria-checked="true"] {
    border-color: var(--text);
  }
</style>
