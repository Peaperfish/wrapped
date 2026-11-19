<script lang="ts">
  // Apps that are never recorded (bundle ids). Add from the apps Wrapped has
  // seen, or type a bundle id.
  import { getAppNames, recordedApps, sendCtl } from "../api";
  import CtlNote from "../CtlNote.svelte";
  import Select from "../Select.svelte";
  import type { Meta } from "../types";

  interface Props {
    meta: Meta;
    onchanged: () => Promise<void>;
  }
  let { meta, onchanged }: Props = $props();

  let busy = $state(false);
  let error = $state<unknown>(null);
  let recorded = $state<string[]>([]);
  let names = $state<Record<string, string>>({});
  let manual = $state("");
  let pick = $state("");

  const excluded = $derived(meta.settings.exclude_apps);
  // Wrapped's own entries (the app bundle, its dashboard, their executables) are not offered.
  const own = (id: string) =>
    id.startsWith("io.github.peaperfish.wrapped") || id.startsWith("exe:wrapped");
  // `recorded` arrives most-used first (by screen time).
  const choices = $derived(recorded.filter((id) => !excluded.includes(id) && !own(id)));

  async function resolve(ids: string[]) {
    const need = ids.filter((id) => !(id in names));
    if (need.length === 0) return;
    try {
      names = { ...names, ...(await getAppNames(need)) };
    } catch {
      /* shown as bundle ids */
    }
  }

  $effect(() => {
    void resolve([...excluded, ...recorded]);
  });

  recordedApps()
    .then((r) => (recorded = r))
    .catch(() => (recorded = []));

  async function save(next: string[]) {
    busy = true;
    error = null;
    try {
      await sendCtl("set_settings", { settings: { "exclude.apps": next } });
      await onchanged();
    } catch (e) {
      error = e;
    } finally {
      busy = false;
    }
  }

  async function add(id: string) {
    const v = id.trim();
    if (!v || excluded.includes(v)) return;
    await save([...excluded, v]);
    manual = "";
    pick = "";
  }

  const label = (id: string) => (names[id] && names[id] !== id ? `${names[id]} (${id})` : id);
</script>

<h3>Exclude list</h3>
<p class="muted lead">
  Apps on this list are never recorded: they leave no trace, not even in debug logs. Time spent in
  them counts as no app.
</p>

<CtlNote {error} />

{#if excluded.length === 0}
  <p class="muted">No apps excluded.</p>
{:else}
  <ul class="list">
    {#each excluded as id (id)}
      <li>
        <span class="name" title={id}>{label(id)}</span>
        <button class="x" aria-label="Stop excluding {label(id)}" disabled={busy} onclick={() => save(excluded.filter((x) => x !== id))}>✕</button>
      </li>
    {/each}
  </ul>
{/if}

<div class="add">
  <label>
    <span class="muted">Add from apps Wrapped has seen</span>
    <span class="inline">
      <Select
        bind:value={pick}
        searchable
        ariaLabel="App to exclude"
        disabled={busy || choices.length === 0}
        placeholder={choices.length === 0 ? "No more apps" : "Choose an app…"}
        options={choices.map((id) => ({ value: id, label: label(id), title: id }))}
      />
      <button disabled={busy || !pick} onclick={() => add(pick)}>Add</button>
    </span>
  </label>
  <label>
    <span class="muted">Or type a bundle id</span>
    <span class="inline">
      <input
        type="text"
        placeholder="com.example.App"
        bind:value={manual}
        disabled={busy}
        onkeydown={(e) => e.key === "Enter" && add(manual)}
      />
      <button disabled={busy || !manual.trim()} onclick={() => add(manual)}>Add</button>
    </span>
  </label>
</div>

<style>
  h3 {
    margin: 0 0 2px;
    font-size: 17px;
  }
  .lead {
    margin: 0 0 14px;
  }
  .list {
    list-style: none;
    margin: 12px 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .list li {
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--field-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 3px 4px 3px 10px;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .x {
    background: transparent;
    border-color: transparent;
    color: var(--text-dim);
    padding: 2px 7px;
  }
  .add {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-top: 16px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
  }
  .inline {
    display: flex;
    gap: 6px;
  }
  .inline :global(.sel),
  .inline input {
    flex: 1;
    min-width: 0;
  }
  input[type="text"] {
    background: var(--field-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 4px 8px;
    user-select: text;
    -webkit-user-select: text;
  }
</style>
