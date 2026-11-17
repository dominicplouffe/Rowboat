<script lang="ts">
  // The panel beside the grid: a JSON viewer for the selected cell, or the
  // selected row as a form (record view), for wide tables.
  import { formatCell } from '../lib/cells';
  import { settings } from '../lib/settings.svelte';
  import type { Column } from '../lib/types';
  import Icon from './Icon.svelte';
  import JsonNode from './JsonNode.svelte';

  let {
    mode,
    columns,
    row,
    column,
    onclose,
  }: {
    mode: 'json' | 'record';
    columns: Column[];
    /** The selected row's values (with staged edits applied). */
    row: (string | null)[];
    /** The selected column, for the JSON viewer. */
    column: number;
    onclose: () => void;
  } = $props();

  let search = $state('');
  let raw = $state(false);
  let notice = $state<string | null>(null);

  const text = $derived(row[column] ?? '');
  const parsed = $derived.by(() => {
    try {
      return { ok: true as const, value: JSON.parse(text) as unknown };
    } catch {
      return { ok: false as const, value: null };
    }
  });
  const pretty = $derived(parsed.ok ? JSON.stringify(parsed.value, null, 2) : text);
  const results = $derived(settings.value?.results);

  async function copy(value: string, what: string) {
    try {
      await navigator.clipboard.writeText(value);
      notice = `Copied ${what}`;
    } catch {
      notice = 'Could not copy to the clipboard.';
    }
    setTimeout(() => (notice = null), 1800);
  }
</script>

<aside class="panel" aria-label={mode === 'json' ? 'JSON viewer' : 'Record view'}>
  <header>
    <span class="title">{mode === 'json' ? `JSON · ${columns[column]?.name ?? ''}` : 'Record'}</span
    >
    <span class="spacer"></span>
    {#if mode === 'json'}
      <button class="btn ghost" aria-pressed={raw} onclick={() => (raw = !raw)}
        >{raw ? 'Tree' : 'Raw'}</button
      >
      <button class="btn ghost" onclick={() => copy(pretty, 'the value')}>Copy</button>
    {/if}
    <button class="btn ghost icon" onclick={onclose} aria-label="Close panel" title="Close"
      ><Icon name="close" /></button
    >
  </header>

  {#if mode === 'json'}
    {#if !raw && parsed.ok}
      <label class="search">
        <Icon name="search" size={13} />
        <input
          type="search"
          placeholder="Search keys and values"
          aria-label="Search JSON"
          spellcheck="false"
          bind:value={search}
        />
      </label>
      <div class="tree">
        <JsonNode
          name={null}
          value={parsed.value}
          path="$"
          depth={0}
          search={search.trim().toLowerCase()}
          onpick={(path) => copy(path, path)}
        />
      </div>
    {:else}
      <pre class="raw">{raw ? text : pretty}</pre>
    {/if}
  {:else}
    <dl class="record">
      {#each columns as col, i (i)}
        <dt title={col.typeName}>{col.name}</dt>
        <dd class:null={row[i] === null}>
          {row[i] === null
            ? (results?.nullText ?? 'NULL')
            : results
              ? formatCell(row[i]!, col, { ...results, maxCellChars: 100_000 })
              : row[i]}
        </dd>
      {/each}
    </dl>
  {/if}
  {#if notice}<p class="notice" role="status">{notice}</p>{/if}
</aside>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    width: 340px;
    flex: none;
    min-height: 0;
    border-left: 1px solid var(--border);
    background: var(--bg);
  }
  header {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 34px;
    padding: 0 4px 0 12px;
    flex: none;
    border-bottom: 1px solid var(--border);
  }
  .title {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .spacer {
    flex: 1;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    margin: 8px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    color: var(--text-muted);
    flex: none;
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
  .tree,
  .raw,
  .record {
    flex: 1;
    min-height: 0;
    margin: 0;
    overflow: auto;
    padding: 4px 8px 12px;
  }
  .raw {
    font: 12px/1.5 var(--editor-font);
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    -webkit-user-select: text;
  }
  .record {
    padding: 8px 12px 12px;
  }
  dt {
    margin-top: 8px;
    font-size: 11px;
    font-weight: 700;
    color: var(--text-muted);
  }
  dd {
    margin: 1px 0 0;
    font: 12px/1.5 var(--grid-font);
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    -webkit-user-select: text;
  }
  dd.null {
    color: var(--text-faint);
    font-style: italic;
  }
  .notice {
    margin: 0;
    padding: 5px 12px;
    font-size: 12px;
    border-top: 1px solid var(--border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
