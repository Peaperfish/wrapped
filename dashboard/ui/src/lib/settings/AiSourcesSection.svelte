<script lang="ts">
  // The folders AI usage is read from: every supported tool with its status,
  // an on / off switch per folder, "Scan now", and custom folders.
  import { onDestroy, tick } from "svelte";
  import { pickFolders, sendCtl } from "../api";
  import CtlNote from "../CtlNote.svelte";
  import { fmtDay } from "../dates";
  import { fmtValue } from "../format";
  import Toggle from "../Toggle.svelte";
  import type { AiSourceMeta, AiToolMeta, Meta } from "../types";

  interface Props {
    meta: Meta;
    onchanged: () => Promise<void>;
  }
  let { meta, onchanged }: Props = $props();

  let busy = $state(false);
  let error = $state<unknown>(null);
  let notice = $state<string | null>(null);
  let addTool = $state("claude");

  const collectorOn = $derived(meta.collectors.find((c) => c.name === "ai_usage")?.enabled ?? false);
  const rows = $derived(
    meta.ai.tools.map((t) => ({ tool: t, sources: meta.ai.sources.filter((s) => s.tool === t.id) })),
  );

  const pad = (n: number) => String(n).padStart(2, "0");
  const date = (ms: number | null): string => {
    if (ms === null) return "";
    const d = new Date(ms);
    return fmtDay(`${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`);
  };

  function status(t: AiToolMeta, s: AiSourceMeta): string {
    if (!s.enabled) return "Off";
    switch (s.state) {
      case "missing":
        return s.last_seen_ms === null ? "Not found" : `Not found, last seen ${date(s.last_seen_ms)}`;
      case "error":
        return "Could not be read";
      case "new":
        return "Not scanned yet";
    }
    if (!t.tokens) return "OK, sessions only (no token counts on disk)";
    return `OK, ${fmtValue(s.files, "count")} log ${s.files === 1 ? "file" : "files"}`;
  }

  async function run(f: () => Promise<unknown>, after?: string) {
    busy = true;
    error = null;
    notice = null;
    try {
      await f();
      if (after) notice = after;
      await onchanged();
    } catch (e) {
      error = e;
    } finally {
      busy = false;
    }
  }

  const setEnabled = (s: AiSourceMeta, enabled: boolean) =>
    run(() => sendCtl("ai_set_source", { id: s.id, enabled }));
  const remove = (s: AiSourceMeta) => run(() => sendCtl("ai_remove_source", { id: s.id }));

  async function addFolder() {
    const [path] = await pickFolders(`Folder with ${meta.ai.tools.find((t) => t.id === addTool)?.title} logs`, false).catch(
      () => [],
    );
    if (!path) return;
    await run(() => sendCtl("ai_add_source", { tool: addTool, path }), "Folder added; reading it now.");
  }

  const scanStamp = (): string => meta.ai.sources.map((s) => s.last_scan_ms ?? 0).join(",");
  let alive = true;
  onDestroy(() => (alive = false));

  async function scanNow() {
    const before = scanStamp();
    await run(() => sendCtl("scan", { what: "ai" }), "Scanning in the background.");
    // The scan runs in the background: look every second until a source's
    // last scan time changes, for at most a minute.
    for (let i = 0; i < 60 && alive; i++) {
      await new Promise((r) => setTimeout(r, 1000));
      if (!alive) return;
      await onchanged().catch(() => {});
      await tick();
      if (scanStamp() !== before) {
        notice = "Scan finished.";
        return;
      }
    }
    if (alive) notice = "Still scanning; check back in a minute.";
  }
</script>

<h3>AI sources</h3>
<p class="muted lead">
  Wrapped counts tokens from the logs AI coding tools keep on this Mac: time, tool, model, token counts
  and the project folder name. Never the text of a prompt or response.
</p>

<CtlNote {error} />

{#if !collectorOn}
  <p class="muted">AI usage is off (turn it on in Tracking).</p>
{/if}

<div class="bar">
  <button disabled={busy || !collectorOn} onclick={scanNow}>Scan now</button>
  {#if notice}<span class="ok small">{notice}</span>{/if}
</div>

<ul class="tools">
  {#each rows as { tool, sources } (tool.id)}
    <li class:absent={sources.length === 0}>
      <div class="head">
        <h4>{tool.title}</h4>
        {#if sources.length === 0}
          <span class="faint small">Not installed</span>
        {:else if !tool.tested}
          <span class="badge small" title="Written from the tool's documentation; not yet checked against real logs">untested</span>
        {/if}
      </div>
      {#if sources.length === 0}
        <p class="faint small">{tool.note}</p>
      {/if}
      {#each sources as s (s.id)}
        <div class="src">
          <div class="text">
            <p class="path" title={s.path}>{s.path}</p>
            <p class="small" class:warn={s.enabled && (s.state === "missing" || s.state === "error")} class:muted={!(s.enabled && (s.state === "missing" || s.state === "error"))}>
              {status(tool, s)}{s.builtin ? "" : " · added by you"}
            </p>
          </div>
          {#if !s.builtin}
            <button class="small" disabled={busy} onclick={() => remove(s)} aria-label="Remove {s.path}">Remove</button>
          {/if}
          <Toggle label="Read {tool.title} from {s.path}" checked={s.enabled} disabled={busy} onchange={(on) => setEnabled(s, on)} />
        </div>
      {/each}
      {#if sources.length > 0 && tool.note}
        <p class="faint small">{tool.note}</p>
      {/if}
    </li>
  {/each}
</ul>

<section>
  <h4>Add a folder</h4>
  <p class="muted small">For logs kept somewhere else (a backup, another account). Choose the tool whose format they use.</p>
  <div class="add">
    <select bind:value={addTool} disabled={busy} aria-label="Log format">
      {#each meta.ai.tools as t (t.id)}
        <option value={t.id}>{t.title}</option>
      {/each}
    </select>
    <button disabled={busy} onclick={addFolder}>Choose folder…</button>
  </div>
</section>

<style>
  h3 {
    margin: 0 0 2px;
    font-size: 17px;
  }
  h4 {
    margin: 0;
    font-size: 14px;
  }
  .lead {
    margin: 0 0 14px;
  }
  .bar {
    display: flex;
    gap: 10px;
    align-items: center;
    margin-bottom: 8px;
  }
  .tools {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .tools > li {
    border-top: 1px solid var(--divider);
    padding: 12px 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .tools > li.absent {
    opacity: 0.55;
  }
  .head {
    display: flex;
    gap: 10px;
    align-items: baseline;
  }
  .badge {
    color: var(--warn);
    border: 1px solid var(--card-border);
    border-radius: 8px;
    padding: 0 6px;
  }
  .src {
    display: flex;
    gap: 10px;
    align-items: center;
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .text p {
    margin: 0;
  }
  .path {
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  p {
    margin: 0;
  }
  .small {
    font-size: 12px;
  }
  .warn {
    color: var(--warn);
  }
  .ok {
    color: var(--accent);
  }
  section {
    border-top: 1px solid var(--divider);
    padding: 14px 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .add {
    display: flex;
    gap: 8px;
  }
  select {
    background: var(--field-bg);
    color: var(--text);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 5px 8px;
    font: inherit;
  }
</style>
