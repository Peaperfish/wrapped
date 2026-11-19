<script lang="ts">
  // Export everything (JSON file or a folder of CSVs) and delete all data.
  import { exportData, pickFolders, pickSavePath, sendCtl } from "../api";
  import CtlNote from "../CtlNote.svelte";
  import { errorText } from "../api";
  import type { ExportResult, Meta } from "../types";

  interface Props {
    meta: Meta;
    onchanged: () => Promise<void>;
  }
  let { meta, onchanged }: Props = $props();

  let busy = $state(false);
  let error = $state<unknown>(null);
  let exported = $state<ExportResult | null>(null);
  let exportError = $state<string | null>(null);
  let confirming = $state(false);
  let typed = $state("");
  let deleted = $state(false);

  async function exportJson() {
    exportError = null;
    exported = null;
    const path = await pickSavePath(`wrapped-export-${meta.today}.json`).catch(() => null);
    if (!path) return;
    busy = true;
    try {
      exported = await exportData("json", path);
    } catch (e) {
      exportError = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function exportCsv() {
    exportError = null;
    exported = null;
    const [dir] = await pickFolders("Folder for the CSV files", false).catch(() => []);
    if (!dir) return;
    busy = true;
    try {
      exported = await exportData("csv", dir);
    } catch (e) {
      exportError = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function deleteAll() {
    busy = true;
    error = null;
    try {
      await sendCtl("delete_all", { confirm: "delete" });
      deleted = true;
      confirming = false;
      typed = "";
      await onchanged();
    } catch (e) {
      error = e;
    } finally {
      busy = false;
    }
  }
</script>

<h3>Data</h3>

<section>
  <h4>Export</h4>
  <p class="muted">Everything Wrapped has recorded, as one JSON file or a folder with one CSV per table. Settings are included.</p>
  <div class="buttons">
    <button disabled={busy} onclick={exportJson}>Export JSON…</button>
    <button disabled={busy} onclick={exportCsv}>Export CSV…</button>
  </div>
  {#if exported}
    <p class="ok small">
      Exported {exported.sessions.toLocaleString()} sessions, {exported.daily_metrics.toLocaleString()} daily
      numbers and {exported.items.toLocaleString()} items to <span class="num">{exported.path}</span>.
    </p>
  {/if}
  {#if exportError}<p class="error small">{exportError}</p>{/if}
</section>

<section>
  <h4>Delete all data</h4>
  <p class="muted">
    Deletes everything Wrapped has recorded: screen time, git, photos, disk and AI usage. Your
    settings stay, and recording continues.
  </p>
  {#if !confirming}
    <div><button class="danger" disabled={busy} onclick={() => { confirming = true; deleted = false; }}>Delete all data…</button></div>
  {:else}
    <div class="confirm" role="group" aria-label="Confirm deleting all data">
      <p>
        <strong>This cannot be undone.</strong> AI usage numbers are rebuilt from the tools' own log
        files, and those tools delete their logs after a while (Claude Code keeps 30 days by default).
        <strong>Usage whose original logs are already gone cannot be re-imported.</strong> Logs that
        still exist are imported again by the next scan.
      </p>
      <label>
        <span>Type <strong>delete</strong> to confirm</span>
        <input type="text" bind:value={typed} autocomplete="off" spellcheck="false" onkeydown={(e) => e.key === "Enter" && typed.trim().toLowerCase() === "delete" && deleteAll()} />
      </label>
      <div class="buttons">
        <button class="danger" disabled={busy || typed.trim().toLowerCase() !== "delete"} onclick={deleteAll}>
          Delete everything
        </button>
        <button disabled={busy} onclick={() => { confirming = false; typed = ""; }}>Cancel</button>
      </div>
    </div>
  {/if}
  {#if deleted}<p class="ok small">All data deleted.</p>{/if}
  <CtlNote {error} />
</section>

<style>
  h3 {
    margin: 0 0 14px;
    font-size: 17px;
  }
  section {
    border-top: 1px solid var(--divider);
    padding: 14px 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h4 {
    margin: 0;
    font-size: 14px;
  }
  p {
    margin: 0;
  }
  .small {
    font-size: 12px;
  }
  .ok {
    color: var(--accent);
  }
  .buttons {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .confirm {
    background: rgba(var(--danger-rgb), 0.08);
    border: 1px solid rgba(var(--danger-rgb), 0.35);
    border-radius: var(--radius-sm);
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
  }
  input[type="text"] {
    background: var(--field-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 5px 8px;
    max-width: 220px;
    user-select: text;
    -webkit-user-select: text;
  }
</style>
