<script lang="ts">
  // A list of folders with remove buttons and "Add folder…" (native picker).
  import { pickFolders } from "./api";

  interface Props {
    folders: string[];
    title: string;
    /** Shown when the list is empty. */
    emptyText?: string;
    disabled?: boolean;
    onchange: (folders: string[]) => void;
  }
  let { folders, title, emptyText = "No folders", disabled = false, onchange }: Props = $props();

  async function add() {
    const picked = await pickFolders(title).catch(() => []);
    const next = [...folders];
    for (const p of picked) if (!next.includes(p)) next.push(p);
    if (next.length !== folders.length) onchange(next);
  }
</script>

<div class="folders">
  {#if folders.length === 0}
    <p class="muted none">{emptyText}</p>
  {:else}
    <ul>
      {#each folders as f (f)}
        <li>
          <span class="path num" title={f}>{f}</span>
          <button
            class="x"
            aria-label="Remove {f}"
            title="Remove"
            {disabled}
            onclick={() => onchange(folders.filter((x) => x !== f))}>✕</button
          >
        </li>
      {/each}
    </ul>
  {/if}
  <div><button {disabled} onclick={add}>Add folder…</button></div>
</div>

<style>
  .folders {
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-start;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  li {
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--field-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding: 3px 4px 3px 10px;
  }
  .path {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .x {
    border-color: transparent;
    background: transparent;
    color: var(--text-dim);
    padding: 2px 7px;
  }
  .none {
    margin: 0;
    font-size: 12px;
  }
</style>
