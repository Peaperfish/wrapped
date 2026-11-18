<script lang="ts">
  // A value with its change against the comparison period: arrow and colour,
  // or "new" when the comparison period had none.
  import type { ComparisonJson } from "./types";
  import { fmtPct } from "./format";

  interface Props {
    value: string;
    label?: string;
    cmp?: ComparisonJson;
    /** Smaller type, for several in one card. */
    small?: boolean;
  }
  let { value, label, cmp, small = false }: Props = $props();

  const dir = $derived(
    !cmp ? "flat" : cmp.pct === null ? (cmp.current > 0 ? "up" : "flat") : cmp.pct > 0 ? "up" : cmp.pct < 0 ? "down" : "flat",
  );
  const arrow = $derived(dir === "up" ? "▲" : dir === "down" ? "▼" : "•");
  const tip = $derived(
    cmp ? `Now ${cmp.current.toLocaleString("en-US")} · before ${cmp.previous.toLocaleString("en-US")}` : "",
  );
</script>

<div class="big" class:small>
  {#if label}<div class="label muted">{label}</div>{/if}
  <div class="row">
    <span class="value num">{value}</span>
    {#if cmp}
      <span class="change num {dir}" title={tip}>{arrow} {cmp.pct === null && cmp.current === 0 ? "0%" : fmtPct(cmp.pct)}</span>
    {/if}
  </div>
</div>

<style>
  .big {
    min-width: 0;
  }
  .label {
    font-size: 12px;
  }
  .row {
    display: flex;
    align-items: baseline;
    gap: 10px;
    flex-wrap: wrap;
  }
  .value {
    font-size: 30px;
    font-weight: 600;
    line-height: 1.15;
  }
  .small .value {
    font-size: 20px;
  }
  .change {
    font-size: 12px;
  }
  .up {
    color: var(--accent);
  }
  .down {
    color: var(--danger);
  }
  .flat {
    color: var(--text-faint);
  }
</style>
