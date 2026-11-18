<script lang="ts">
  // Pick the author emails whose commits count as yours (like
  // `wrapped-collector git emails`). Looks in the given folders (read-only).
  import { errorText, gitEmails } from "./api";
  import type { GitEmail } from "./types";

  interface Props {
    folders: string[];
    /** Lowercase emails currently chosen. */
    selected: string[];
    onchange: (emails: string[]) => void;
  }
  let { folders, selected, onchange }: Props = $props();

  let found = $state.raw<GitEmail[] | null>(null);
  let error = $state<string | null>(null);
  let seq = 0;

  $effect(() => {
    const fs = folders;
    const mine = ++seq;
    found = null;
    error = null;
    if (fs.length === 0) {
      found = [];
      return;
    }
    gitEmails(fs)
      .then((r) => {
        if (mine !== seq) return;
        found = r;
        // First look: tick the owner's own email when nothing is chosen yet.
        if (selected.length === 0) {
          const own = r.filter((e) => e.mine).map((e) => e.email);
          if (own.length > 0) onchange(own);
        }
      })
      .catch((e) => {
        if (mine === seq) error = errorText(e);
      });
  });

  function toggle(email: string, on: boolean) {
    onchange(on ? [...new Set([...selected, email])] : selected.filter((e) => e !== email));
  }
</script>

<div class="emails">
  <p class="label muted">Commits by these author emails are yours</p>
  {#if error}
    <p class="error">{error}</p>
  {:else if found === null}
    <p class="muted small">Looking for author emails in your folders…</p>
  {:else if found.length === 0}
    <p class="muted small">No repositories with commits found in these folders.</p>
  {:else}
    <ul>
      {#each found as e (e.email)}
        <li>
          <label>
            <input
              type="checkbox"
              checked={selected.includes(e.email)}
              onchange={(ev) => toggle(e.email, ev.currentTarget.checked)}
            />
            <span class="addr num">{e.email}</span>
            <span class="faint small">{e.commits} commits · {e.repos} repos{e.mine ? " · your git config" : ""}</span>
          </label>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .emails {
    width: 100%;
  }
  .label {
    margin: 0 0 4px;
    font-size: 12px;
  }
  .small {
    font-size: 12px;
    margin: 0;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 150px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  label {
    display: flex;
    gap: 8px;
    align-items: baseline;
    padding: 2px 4px;
    border-radius: 6px;
    cursor: pointer;
  }
  label:hover {
    background: var(--hover-bg);
  }
  .addr {
    font-size: 12px;
  }
  input[type="checkbox"] {
    accent-color: var(--accent);
    align-self: center;
  }
</style>
