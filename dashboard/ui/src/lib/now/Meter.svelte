<script lang="ts">
  // A horizontal bar: a fixed track with a fill that is scaled (transform), so
  // updating it never changes layout.
  interface Props {
    /** 0..1; null draws an empty track. */
    fraction: number | null;
    height?: number;
    label?: string;
  }
  let { fraction, height = 8, label }: Props = $props();
  const f = $derived(fraction == null ? 0 : Math.min(Math.max(fraction, 0), 1));
</script>

<div
  class="track"
  style:height="{height}px"
  role="meter"
  aria-label={label}
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={fraction == null ? undefined : Math.round(f * 100)}
>
  <div class="fill" style:transform="scaleX({f})"></div>
</div>

<style>
  .track {
    background: var(--cell-empty);
    border-radius: 999px;
    overflow: hidden;
    min-width: 0;
  }
  .fill {
    height: 100%;
    width: 100%;
    background: var(--accent);
    border-radius: 999px;
    transform-origin: left center;
    transition: transform 0.9s linear;
    will-change: transform;
  }
  @media (prefers-reduced-motion: reduce) {
    .fill {
      transition: none;
    }
  }
</style>
