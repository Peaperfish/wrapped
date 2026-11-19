<script lang="ts">
  // A 60-second sparkline: the newest value is at the right edge; gaps (null)
  // break the line. Only the path's `d` attribute changes each second.
  import { HISTORY, type Series } from "./sampler.svelte";

  interface Props {
    /** One or two series (the second is drawn dimmer). */
    series: Series[];
    /** Fixed top of the scale (e.g. 100 for percentages); auto when omitted. */
    max?: number;
    label: string;
  }
  let { series, max, label }: Props = $props();

  const W = 240;
  const H = 40;
  const PAD = 2;

  const top = $derived(
    max ?? Math.max(1, ...series.flatMap((s) => s.map((v) => v ?? 0))),
  );

  function path(s: Series): string {
    const n = s.length;
    let d = "";
    let pen = false;
    for (let i = 0; i < n; i++) {
      const v = s[i];
      if (v == null) {
        pen = false;
        continue;
      }
      const x = ((HISTORY - n + i) / (HISTORY - 1)) * W;
      const y = H - PAD - (Math.min(v, top) / top) * (H - 2 * PAD);
      d += `${pen ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`;
      pen = true;
    }
    return d;
  }
</script>

<svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="img" aria-label={label}>
  <title>{label}</title>
  <line x1="0" y1={H - PAD} x2={W} y2={H - PAD} class="base" />
  {#each series as s, i (i)}
    <path d={path(s)} class="line" class:second={i === 1} vector-effect="non-scaling-stroke" />
  {/each}
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: 40px;
  }
  .base {
    stroke: var(--divider);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  .line {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.6;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .line.second {
    stroke: var(--text-dim);
  }
</style>
