<script lang="ts">
  // First-launch checklist (DESIGN.md §12). Shown instead of the dashboard
  // until the user confirms; nothing is recorded before that. The choices go
  // to the collector in one request (`finish_setup`).
  import { untrack } from "svelte";
  import { sendCtl } from "./api";
  import CtlNote from "./CtlNote.svelte";
  import FolderList from "./FolderList.svelte";
  import GitEmails from "./GitEmails.svelte";
  import Toggle from "./Toggle.svelte";
  import type { Meta } from "./types";

  interface Props {
    meta: Meta;
    ondone: () => void;
  }
  let { meta, ondone }: Props = $props();

  // Defaults come from the collectors themselves (App usage, Active / idle
  // and AI usage on; Git, Photos, Disk & Memory off).
  // Starting values only: later changes to `meta` do not reset the user's choices.
  const init = untrack(() => meta);
  let ticked = $state<Record<string, boolean>>(
    Object.fromEntries(init.collectors.map((c) => [c.name, c.default_on])),
  );
  let gitFolders = $state<string[]>([...init.settings.git_folders]);
  let gitEmails = $state<string[]>([...(init.settings.git_author_emails ?? [])]);
  let photosFolders = $state<string[]>([...init.settings.photos_folders]);
  let busy = $state(false);
  let error = $state<unknown>(null);

  async function start() {
    busy = true;
    error = null;
    try {
      const settings: Record<string, unknown> = {};
      if (ticked.git) {
        settings["git.folders"] = gitFolders;
        settings["git.author_emails"] = gitEmails;
      }
      if (ticked.photos) settings["photos.folders"] = photosFolders;
      await sendCtl("finish_setup", { settings, collectors: ticked });
      ondone();
    } catch (e) {
      error = e;
    } finally {
      busy = false;
    }
  }
</script>

<section class="setup card" aria-labelledby="setup-title">
  <header>
    <h1 id="setup-title">Welcome to Wrapped</h1>
    <p class="muted">
      Choose what Wrapped records. Everything stays on this Mac, and <strong>nothing is recorded until
      you press Start recording</strong>. You can change any of this later in Settings.
    </p>
  </header>

  <ul class="rows">
    {#each meta.collectors as c (c.name)}
      <li class:on={ticked[c.name]}>
        <div class="row">
          <div class="text">
            <h3>{c.title}</h3>
            <p class="muted">{c.description}</p>
            <p class="faint meta">Permission: {c.permissions} · Cost: {c.cost}</p>
          </div>
          <Toggle
            label="Record {c.title}"
            checked={ticked[c.name] ?? false}
            onchange={(on) => (ticked[c.name] = on)}
          />
        </div>

        {#if c.name === "git" && ticked.git}
          <div class="more">
            <FolderList
              folders={gitFolders}
              title="Folders with your git repositories"
              emptyText="No folders: nothing will be scanned"
              onchange={(f) => (gitFolders = f)}
            />
            <GitEmails folders={gitFolders} selected={gitEmails} onchange={(e) => (gitEmails = e)} />
          </div>
        {:else if c.name === "photos" && ticked.photos}
          <div class="more">
            <FolderList
              folders={photosFolders}
              title="Folders with your photos"
              emptyText="No folders: nothing will be scanned"
              onchange={(f) => (photosFolders = f)}
            />
          </div>
        {/if}
      </li>
    {/each}
  </ul>

  <p class="never faint">
    Never recorded: keystrokes, screen contents, window titles, file contents, and the text of any
    prompt or response.
  </p>

  <CtlNote {error} />

  <footer>
    <button class="primary" disabled={busy} onclick={start}>
      {busy ? "Starting…" : "Start recording"}
    </button>
  </footer>
</section>

<style>
  .setup {
    max-width: 760px;
    width: 100%;
    margin: 24px auto;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  h1 {
    margin: 0 0 4px;
    font-size: 22px;
  }
  header p {
    margin: 0;
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .rows li {
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 12px 14px;
    background: var(--card-bg);
  }
  .rows li.on {
    border-color: rgba(var(--accent-rgb), 0.35);
  }
  .row {
    display: flex;
    gap: 16px;
    align-items: flex-start;
    justify-content: space-between;
  }
  h3 {
    margin: 0 0 2px;
    font-size: 14px;
  }
  .text p {
    margin: 0;
    font-size: 13px;
  }
  .text .meta {
    margin-top: 4px;
    font-size: 12px;
  }
  .more {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--divider);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .never {
    margin: 0;
    font-size: 12px;
  }
  footer {
    display: flex;
    justify-content: flex-end;
  }
  footer button {
    padding: 8px 18px;
  }
</style>
