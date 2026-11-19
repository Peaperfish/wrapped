<script lang="ts">
  // Collector on / off switches (turning one off asks keep or delete), and pause / resume.
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
  /** The collector whose "turn off" is waiting for keep / delete. */
  let confirming = $state<string | null>(null);

  async function run(cmd: string, args: unknown) {
    busy = true;
    error = null;
    try {
      await sendCtl(cmd, args);
      confirming = null;
      await onchanged();
    } catch (e) {
      error = e;
    } finally {
      busy = false;
    }
  }

  const setCollector = (name: string, enabled: boolean, deleteData = false) =>
    run("set_collector", { name, enabled, delete_data: deleteData });
  const pause = (mode: "off" | "hour" | "tomorrow" | "indefinite") => run("pause", { mode });

  const paused = $derived(meta.settings.pause !== "off");
  const pauseText = $derived.by(() => {
    const p = meta.settings.pause;
    if (p === "indefinite") return "Paused until you resume";
    if (p === "until" && meta.settings.pause_until_ms != null) {
      const t = new Date(meta.settings.pause_until_ms);
      const sameDay = t.toDateString() === new Date().toDateString();
      const hm = t.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
      return `Paused until ${sameDay ? "" : t.toLocaleDateString([], { weekday: "short" }) + " "}${hm}`;
    }
    return "Recording";
  });
</script>

<h3>Tracking</h3>
<p class="muted lead">Turn what Wrapped records on or off. Changes apply at once.</p>

<div class="pause">
  <span class="state" class:paused>{pauseText}</span>
  <div class="buttons">
    <button disabled={busy} onclick={() => pause("hour")}>Pause 1 hour</button>
    <button disabled={busy} onclick={() => pause("tomorrow")}>Until tomorrow</button>
    <button disabled={busy} onclick={() => pause("indefinite")}>Indefinitely</button>
    <button disabled={busy || !paused} onclick={() => pause("off")}>Resume</button>
  </div>
</div>

<CtlNote {error} />

<ul class="list">
  {#each meta.collectors as c (c.name)}
    <li>
      <div class="row">
        <div class="text">
          <strong>{c.title}</strong>
          <p class="muted">{c.description}</p>
          <p class="faint small">Permission: {c.permissions} · Cost: {c.cost}</p>
        </div>
        <Toggle
          label="Record {c.title}"
          checked={c.enabled}
          disabled={busy}
          onchange={(on) => (on ? setCollector(c.name, true) : (confirming = c.name))}
        />
      </div>
      {#if confirming === c.name}
        <div class="confirm" role="group" aria-label="Turn off {c.title}">
          <p>Turn off {c.title}? Keep what it has recorded, or delete it?</p>
          <div class="buttons">
            <button disabled={busy} onclick={() => setCollector(c.name, false, false)}>Turn off, keep data</button>
            <button class="danger" disabled={busy} onclick={() => setCollector(c.name, false, true)}>
              Turn off and delete its data
            </button>
            <button disabled={busy} onclick={() => (confirming = null)}>Cancel</button>
          </div>
        </div>
      {/if}
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
  .pause {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    background: var(--field-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 10px 12px;
    margin-bottom: 12px;
  }
  .state {
    font-weight: 600;
    color: var(--accent);
  }
  .state.paused {
    color: var(--warn);
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .list {
    list-style: none;
    margin: 12px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .list li {
    padding: 12px 0;
    border-top: 1px solid var(--divider);
  }
  .row {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    align-items: flex-start;
  }
  .text p {
    margin: 2px 0 0;
    font-size: 13px;
  }
  .text .small {
    font-size: 12px;
  }
  .confirm {
    margin-top: 10px;
    background: rgba(var(--warn-rgb), 0.08);
    border: 1px solid rgba(var(--warn-rgb), 0.3);
    border-radius: var(--radius-sm);
    padding: 10px 12px;
  }
  .confirm p {
    margin: 0 0 8px;
  }
</style>
