<script lang="ts">
  // 24 bars, one per local hour 00..23, labelled every 6 hours. Hovering a
  // bar shows its hour and value.
  interface Props {
    hours: number[];
    format: (v: number) => string;
  }
  let { hours, format }: Props = $props();

  const W = 480;
  const H = 120;
  const PAD_B = 18;
  const slot = W / 24;
  const max = $derived(Math.max(...hours, 0));
  const hh = (h: number): string => String(h).padStart(2, "0");
  let hover = $state<number | null>(null);
</script>

<svg viewBox="0 0 {W} {H}" role="img" aria-label="Screen time by hour of day" onmouseleave={() => (hover = null)}>
  {#each hours as v, h (h)}
    {@const bh = max > 0 ? (v / max) * (H - PAD_B - 6) : 0}
    <g onmouseenter={() => (hover = h)} role="presentation">
      <rect x={h * slot} y="0" width={slot} height={H - PAD_B} fill="transparent" />
      <rect
        x={h * slot + 2}
        y={H - PAD_B - bh}
        width={slot - 4}
        height={bh}
        rx="2"
        fill={hover === h ? "var(--cell-4)" : "var(--cell-3)"}
      />
      <title>{hh(h)}:00 – {hh(h)}:59 · {format(v)}</title>
    </g>
  {/each}
  <line x1="0" x2={W} y1={H - PAD_B + 0.5} y2={H - PAD_B + 0.5} stroke="var(--divider)" />
  {#each [0, 6, 12, 18] as h (h)}
    <text x={h * slot + slot / 2} y={H - 4} text-anchor="middle" fill="var(--text-faint)" font-size="11" class="num">{hh(h)}</text>
  {/each}
</svg>

<style>
  svg {
    width: 100%;
    height: auto;
    display: block;
  }
</style>
