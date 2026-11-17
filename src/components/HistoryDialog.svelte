<script lang="ts">
  import { onMount } from 'svelte';
  import { connections } from '../lib/connections.svelte';
  import { dialogs } from '../lib/dialogs.svelte';
  import { ipc } from '../lib/ipc';
  import { settings } from '../lib/settings.svelte';
  import { tabs } from '../lib/tabs.svelte';
  import type { HistoryEntry } from '../lib/types';
  import { ui } from '../lib/ui.svelte';
  import Icon from './Icon.svelte';

  let search = $state('');
  let onlyThis = $state(!!tabs.active?.connectionId);
  let entries = $state<HistoryEntry[]>([]);
  let loading = $state(true);
  let input: HTMLInputElement;

  const connectionId = $derived(onlyThis ? (tabs.active?.connectionId ?? null) : null);

  let request = 0;
  $effect(() => {
    const [id, text] = [connectionId, search];
    const mine = ++request;
    loading = true;
    // A short pause so each keystroke does not query the history.
    const timer = setTimeout(async () => {
      const found = await ipc.historySearch(id, text).catch(() => []);
      if (mine === request) {
        entries = found;
        loading = false;
      }
    }, 120);
    return () => clearTimeout(timer);
  });

  onMount(() => input.focus());

  const close = () => (ui.historyOpen = false);

  function open(entry: HistoryEntry) {
    tabs.open(settings.value?.execution.defaultRowLimit ?? 1000, entry.sql, {
      connectionId: connections.byId(entry.connectionId) ? entry.connectionId : null,
    });
    close();
  }

  async function clear() {
    const ok = await dialogs.confirm({
      title: 'Clear all query history?',
      message:
        'This removes the history of every connection from this computer. It cannot be undone.',
      confirmLabel: 'Clear history',
      danger: true,
    });
    if (!ok) return;
    await ipc.historyClear();
    entries = [];
  }

  const when = (ms: number) =>
    new Date(ms).toLocaleString(undefined, {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  const duration = (ms: number) => (ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`);

  function onKeydown(event: KeyboardEvent) {
    if (dialogs.confirmRequest) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      close();
    }
  }
</script>

<svelte:window onkeydowncapture={onKeydown} />

<div
  class="backdrop"
  role="presentation"
  onpointerdown={(e) => e.target === e.currentTarget && close()}
>
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Query history">
    <header>
      <label class="search">
        <Icon name="search" size={14} />
        <input
          bind:this={input}
          bind:value={search}
          type="search"
          placeholder="Search query history"
          aria-label="Search query history"
          spellcheck="false"
        />
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={onlyThis} disabled={!tabs.active?.connectionId} />
        This connection only
      </label>
      <button class="btn ghost icon" onclick={close} aria-label="Close" title="Close (Esc)"
        ><Icon name="close" /></button
      >
    </header>

    <div class="list">
      {#each entries as entry (entry.id)}
        <button class="entry" onclick={() => open(entry)} title="Open in a new tab">
          <span class="sql"
            >{entry.sql.length > 400 ? `${entry.sql.slice(0, 400)}…` : entry.sql}</span
          >
          <span class="meta">
            <span>{when(entry.startedAt)}</span>
            <span>{connections.byId(entry.connectionId)?.name ?? 'deleted connection'}</span>
            <span>{duration(entry.elapsedMs)}</span>
            {#if entry.error}
              <span class="failed">failed: {entry.error.slice(0, 80)}</span>
            {:else if entry.rows !== null}
              <span>{entry.rows.toLocaleString('en-US')} rows</span>
            {/if}
          </span>
        </button>
      {:else}
        <p class="empty muted">
          {#if loading}
            Searching…
          {:else if settings.value && !settings.value.privacy.keepHistory}
            Query history is turned off in Settings → Privacy & data.
          {:else if search.trim()}
            Nothing in your history matches “{search.trim()}”.
          {:else}
            Queries you run will be listed here.
          {/if}
        </p>
      {/each}
    </div>

    <footer>
      <span class="muted">Kept on this computer only. Click an entry to open it in a new tab.</span>
      <span class="spacer"></span>
      <button class="btn" onclick={clear} disabled={entries.length === 0}>Clear history…</button>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: grid;
    place-items: center;
    background: rgba(8, 20, 34, 0.42);
  }
  .dialog {
    display: flex;
    flex-direction: column;
    width: min(760px, calc(100vw - 48px));
    height: min(600px, calc(100vh - 48px));
    background: var(--bg);
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    flex: none;
  }
  header {
    border-bottom: 1px solid var(--border);
  }
  footer {
    border-top: 1px solid var(--border);
    font-size: 12px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    color: var(--text-muted);
  }
  .search:focus-within {
    outline: 2px solid var(--focus);
    outline-offset: -1px;
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: none;
    appearance: none;
    -webkit-appearance: none;
    user-select: text;
    -webkit-user-select: text;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    font-size: 12px;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .entry {
    display: block;
    width: 100%;
    padding: 8px 14px;
    border: none;
    border-bottom: 1px solid var(--border);
    background: none;
    text-align: left;
  }
  .entry:hover,
  .entry:focus-visible {
    background: var(--bg-hover);
  }
  .sql {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    font: 12px/1.5 var(--editor-font);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .meta {
    display: flex;
    gap: 12px;
    margin-top: 3px;
    font-size: 11px;
    color: var(--text-faint);
  }
  .failed {
    color: var(--danger);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty {
    margin: 60px 0;
    text-align: center;
  }
  .spacer {
    flex: 1;
  }
</style>
