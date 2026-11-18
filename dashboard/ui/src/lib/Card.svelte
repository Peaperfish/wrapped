<script lang="ts">
  // A card of the grid: S (3 columns), M (6) or L (12). Shows its content, or
  // "Nothing in this range" when it has none.
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    subtitle?: string;
    size: "S" | "M" | "L";
    empty?: boolean;
    children: Snippet;
  }
  let { title, subtitle, size, empty = false, children }: Props = $props();
</script>

<section class="card size-{size}">
  <header>
    <h2>{title}</h2>
    {#if subtitle}<p class="sub faint">{subtitle}</p>{/if}
  </header>
  {#if empty}
    <p class="none muted">Nothing in this range</p>
  {:else}
    <div class="body">{@render children()}</div>
  {/if}
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .size-S {
    grid-column: span 3;
  }
  .size-M {
    grid-column: span 6;
  }
  .size-L {
    grid-column: span 12;
  }
  /* Near the 960 px minimum width the small cards pair up. */
  @media (max-width: 1100px) {
    .size-S {
      grid-column: span 6;
    }
  }
  header {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .sub {
    margin: 0;
    font-size: 12px;
  }
  .none {
    margin: 0;
    padding: 18px 0;
    text-align: center;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 12px;
    flex: 1;
  }
</style>
