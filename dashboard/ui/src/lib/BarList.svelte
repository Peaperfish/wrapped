<script lang="ts">
  // Horizontal bars with a label and a value; the longest bar is the largest
  // value. Bars are SVG; hovering a row shows its full label and value.
  export interface BarItem {
    label: string;
    value: number;
    /** Formatted value. */
    text: string;
    /** Tooltip (defaults to "label: text"). */
    title?: string;
  }
  interface Props {
    items: BarItem[];
  }
  let { items }: Props = $props();
  const max = $derived(Math.max(...items.map((i) => i.value), 0));
</script>

<ul>
  {#each items as it (it.label)}
    <li title={it.title ?? `${it.label}: ${it.text}`}>
      <span class="label">{it.label}</span>
      <span class="val num">{it.text}</span>
      <svg class="bar" viewBox="0 0 100 6" preserveAspectRatio="none" aria-hidden="true">
        <rect x="0" y="0" width="100" height="6" rx="3" fill="var(--cell-empty)" />
        <rect
          x="0"
          y="0"
          width={max > 0 ? Math.max((it.value / max) * 100, 2) : 0}
          height="6"
          rx="3"
          fill="var(--accent)"
        />
        <title>{it.title ?? `${it.label}: ${it.text}`}</title>
      </svg>
    </li>
  {/each}
</ul>

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  li {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 3px 12px;
    min-width: 0;
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .val {
    color: var(--text-dim);
    font-size: 12px;
    align-self: center;
  }
  .bar {
    grid-column: 1 / -1;
    width: 100%;
    height: 6px;
    display: block;
  }
</style>
