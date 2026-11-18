<script lang="ts">
  // Folders Git and Photos look in, and the author emails that count as yours.
  import { sendCtl } from "../api";
  import CtlNote from "../CtlNote.svelte";
  import FolderList from "../FolderList.svelte";
  import GitEmails from "../GitEmails.svelte";
  import type { Meta } from "../types";

  interface Props {
    meta: Meta;
    onchanged: () => Promise<void>;
  }
  let { meta, onchanged }: Props = $props();

  let busy = $state(false);
  let error = $state<unknown>(null);
  /** Emails ticked in the list but not saved yet (saving makes git re-read its history). */
  let pendingEmails = $state<string[] | null>(null);

  const savedEmails = $derived(meta.settings.git_author_emails ?? []);
  const shownEmails = $derived(pendingEmails ?? savedEmails);
  const emailsDirty = $derived(
    pendingEmails !== null &&
      [...pendingEmails].sort().join("\n") !== [...savedEmails].sort().join("\n"),
  );
  const on = (name: string) => meta.collectors.find((c) => c.name === name)?.enabled ?? false;

  async function save(settings: Record<string, unknown>) {
    busy = true;
    error = null;
    try {
      await sendCtl("set_settings", { settings });
      pendingEmails = null;
      await onchanged();
    } catch (e) {
      error = e;
    } finally {
      busy = false;
    }
  }
</script>

<h3>Folders</h3>
<p class="muted lead">Where Wrapped looks for git repositories and photos. Only folder names and dates are read, never contents.</p>

<CtlNote {error} />

<section>
  <h4>Git {#if !on("git")}<span class="off faint">· off (turn it on in Tracking)</span>{/if}</h4>
  <FolderList
    folders={meta.settings.git_folders}
    title="Folders with your git repositories"
    emptyText="No folders: nothing is scanned"
    disabled={busy}
    onchange={(f) => save({ "git.folders": f })}
  />
  <div class="emails">
    <GitEmails
      folders={meta.settings.git_folders}
      selected={shownEmails}
      onchange={(e) => (pendingEmails = e)}
    />
    {#if emailsDirty}
      <p class="warn small">
        Changing which emails count makes Wrapped read your git history again (a minute or so in the
        background). Commits already counted are replaced by the new count.
      </p>
      <div class="buttons">
        <button class="primary" disabled={busy} onclick={() => save({ "git.author_emails": pendingEmails })}>
          Save emails
        </button>
        <button disabled={busy} onclick={() => (pendingEmails = null)}>Cancel</button>
      </div>
    {/if}
  </div>
</section>

<section>
  <h4>Photos {#if !on("photos")}<span class="off faint">· off (turn it on in Tracking)</span>{/if}</h4>
  <FolderList
    folders={meta.settings.photos_folders}
    title="Folders with your photos"
    emptyText="No folders: nothing is scanned"
    disabled={busy}
    onchange={(f) => save({ "photos.folders": f })}
  />
</section>

<style>
  h3 {
    margin: 0 0 2px;
    font-size: 17px;
  }
  .lead {
    margin: 0 0 14px;
  }
  section {
    border-top: 1px solid var(--divider);
    padding: 14px 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h4 {
    margin: 0;
    font-size: 14px;
  }
  .off {
    font-weight: 400;
    font-size: 12px;
  }
  .warn {
    margin: 8px 0;
    color: var(--warn);
  }
  .small {
    font-size: 12px;
  }
  .buttons {
    display: flex;
    gap: 6px;
  }
</style>
