<script lang="ts">
  // Recap notifications: one switch per period, all off by default. Turning
  // one on makes the collector ask macOS for permission (once); if it is
  // denied, this section says where to allow it.
  import { sendCtl } from "../api";
  import CtlNote from "../CtlNote.svelte";
  import Toggle from "../Toggle.svelte";
  import type { Meta } from "../types";

  interface Props {
    meta: Meta;
    onchanged: () => Promise<void>;
  }
  let { meta, onchanged }: Props = $props();

  let busy = $state(false);
  let error = $state<unknown>(null);

  const rows = [
    { kind: "week", title: "Weekly", when: "Mondays from 09:00, for the week before" },
    { kind: "month", title: "Monthly", when: "The 1st from 09:00, for the month before" },
    { kind: "year", title: "Yearly", when: "1 December from 09:00, for the year so far" },
  ] as const;
  const on = (k: "week" | "month" | "year"): boolean =>
    k === "week" ? meta.settings.notify_week : k === "month" ? meta.settings.notify_month : meta.settings.notify_year;
  const anyOn = $derived(meta.settings.notify_week || meta.settings.notify_month || meta.settings.notify_year);
  const permission = $derived(meta.settings.notify_permission);

  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  async function set(kind: string, value: boolean) {
    busy = true;
    error = null;
    try {
      await sendCtl("set_settings", { settings: { [`recaps.notify.${kind}`]: value } });
      await onchanged();
      if (value) {
        // The permission answer arrives a moment later (or after the prompt).
        for (const ms of [1500, 4000, 8000]) {
          await sleep(ms);
          await onchanged();
          if (meta.settings.notify_permission === "granted" || meta.settings.notify_permission === "denied") break;
        }
      }
    } catch (e) {
      error = e;
    } finally {
      busy = false;
    }
  }
</script>

<h3>Recaps</h3>
<p class="muted lead">
  Get a notification when a new recap is ready. It shows one number and opens the recap when clicked;
  nothing leaves this computer.
</p>

<CtlNote {error} />

{#if anyOn && permission === "denied"}
  <p class="note warn" role="status">
    {navigator.userAgent.includes("Windows")
      ? "Allow Wrapped notifications in Settings → System → Notifications."
      : "Allow Wrapped notifications in System Settings → Notifications → Wrapped."}
  </p>
{:else if anyOn && permission === "unavailable"}
  <p class="note" role="status">
    {navigator.userAgent.includes("Windows")
      ? "Windows notifications are unavailable for this copy of Wrapped."
      : "This copy of Wrapped is not running as Wrapped.app, so it cannot show notifications."}
  </p>
{/if}

<ul class="list">
  {#each rows as r (r.kind)}
    <li class="row">
      <div class="text">
        <strong>{r.title}</strong>
        <p class="muted">{r.when}</p>
      </div>
      <Toggle checked={on(r.kind)} label="{r.title} recap notifications" disabled={busy} onchange={(v) => set(r.kind, v)} />
    </li>
  {/each}
</ul>

<style>
  h3 {
    margin: 0 0 2px;
    font-size: 17px;
  }
  .lead {
    margin: 0 0 14px;
  }
  .note {
    margin: 0 0 12px;
    padding: 10px 12px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--card-border);
    background: var(--field-bg);
    font-size: 13px;
  }
  .note.warn {
    color: var(--warn);
    border-color: rgba(var(--warn-rgb), 0.35);
    background: rgba(var(--warn-rgb), 0.08);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 16px;
    padding: 12px 0;
    border-top: 1px solid var(--divider);
  }
  .text p {
    margin: 2px 0 0;
    font-size: 13px;
  }
</style>
