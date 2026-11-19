<script lang="ts">
  // Shows why a request to the collector failed. When Wrapped is not running,
  // offers to open it (changes are made by the collector, never by the dashboard).
  import { CtlError, errorText, openWrapped } from "./api";

  interface Props {
    error: unknown;
  }
  let { error }: Props = $props();

  const notRunning = $derived(error instanceof CtlError && error.code === "not_running");
  let opening = $state(false);

  async function open() {
    opening = true;
    await openWrapped();
    setTimeout(() => (opening = false), 2500);
  }
</script>

{#if error}
  <div class="note" role="alert">
    {#if notRunning}
      <span>Wrapped isn't running, so it can't change settings. Open Wrapped, then try again.</span>
      <button disabled={opening} onclick={open}>{opening ? "Opening…" : "Open Wrapped"}</button>
    {:else}
      <span>{errorText(error)}</span>
    {/if}
  </div>
{/if}

<style>
  .note {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    background: rgba(var(--danger-rgb), 0.1);
    border: 1px solid rgba(var(--danger-rgb), 0.35);
    color: var(--text);
    border-radius: var(--radius-sm);
    padding: 8px 12px;
    font-size: 13px;
  }
</style>
