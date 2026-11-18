<script lang="ts">
  // An SVG arc: `fraction` (0..1) of the circle in the accent colour.
  interface Props {
    fraction: number;
    /** Big text in the middle. */
    center: string;
    /** Small text under it. */
    caption?: string;
    title?: string;
  }
  let { fraction, center, caption, title }: Props = $props();
  const R = 52;
  const C = 2 * Math.PI * R;
  const f = $derived(Math.min(Math.max(fraction, 0), 1));
</script>

<svg viewBox="0 0 140 140" role="img" aria-label={title ?? center}>
  <title>{title ?? center}</title>
  <circle cx="70" cy="70" r={R} fill="none" stroke="var(--cell-empty)" stroke-width="12" />
  <circle
    cx="70"
    cy="70"
    r={R}
    fill="none"
    stroke="var(--accent)"
    stroke-width="12"
    stroke-linecap="round"
    stroke-dasharray="{f * C} {C}"
    transform="rotate(-90 70 70)"
  />
  <text x="70" y={caption ? 70 : 76} text-anchor="middle" class="num" fill="var(--text)" font-size="22" font-weight="600"
    >{center}</text
  >
  {#if caption}
    <text x="70" y="90" text-anchor="middle" fill="var(--text-dim)" font-size="11">{caption}</text>
  {/if}
</svg>

<style>
  svg {
    width: 132px;
    height: 132px;
    flex: none;
  }
</style>
