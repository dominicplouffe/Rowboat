<script lang="ts">
  import { connections } from '../lib/connections.svelte';
  import { cancel } from '../lib/runner';
  import type { Tab } from '../lib/tabs.svelte';
  import type { DbError } from '../lib/types';
  import Grid from './Grid.svelte';
  import Icon from './Icon.svelte';
  import RowLimitMenu from './RowLimitMenu.svelte';

  let { tab }: { tab: Tab } = $props();

  const shown = $derived(tab.runs[tab.shown]);
  const result = $derived(shown?.result);

  const ERROR_TITLES: Record<DbError['kind'], string> = {
    syntax: 'Syntax error',
    schema: 'Unknown object',
    permission: 'Not allowed',
    connection: 'Connection problem',
    cancelled: 'Cancelled',
    timeout: 'Timed out',
    'needs-confirmation': 'Needs confirmation',
    'unknown-host-key': 'Unknown SSH host',
    other: 'Error',
  };

  const count = (n: number) => n.toLocaleString('en-US');
  const duration = (ms: number) => (ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(2)} s`);

  const summary = $derived.by(() => {
    if (!result) return '';
    const parts: string[] = [];
    if (result.columns.length > 0) {
      parts.push(`${count(result.rows.length)} ${result.rows.length === 1 ? 'row' : 'rows'}`);
      if (result.moreAvailable) parts.push('more available');
    } else if (result.rowsAffected !== null) {
      parts.push(
        `${count(result.rowsAffected)} ${result.rowsAffected === 1 ? 'row' : 'rows'} affected`,
      );
    }
    parts.push(duration(result.elapsedMs));
    return parts.join(' · ');
  });

  /** Line and column of the error within its statement. */
  const where = $derived.by(() => {
    const position = shown?.error?.position;
    if (!shown || !position) return null;
    const before = [...shown.sql].slice(0, position - 1).join('');
    const lines = before.split('\n');
    return { line: lines.length, column: [...lines[lines.length - 1]].length + 1 };
  });

  // A running timer for the statement in flight.
  let elapsed = $state(0);
  $effect(() => {
    if (!tab.running) return;
    const started = Date.now();
    elapsed = 0;
    const timer = setInterval(() => (elapsed = Date.now() - started), 100);
    return () => clearInterval(timer);
  });

  const statusIcon = { pending: '·', running: '…', done: '✓', failed: '✕', skipped: '–' } as const;
</script>

<div class="results">
  <div class="toolbar">
    <span class="section">Results</span>
    {#if tab.runs.length > 1}
      <div class="statements" role="tablist" aria-label="Statements in this run">
        {#each tab.runs as statement, i (i)}
          <button
            role="tab"
            class={statement.status}
            class:active={i === tab.shown}
            aria-selected={i === tab.shown}
            title={statement.sql.slice(0, 300)}
            onclick={() => (tab.shown = i)}
          >
            <span aria-hidden="true">{statusIcon[statement.status]}</span>
            {i + 1}
          </button>
        {/each}
      </div>
    {/if}
    <span class="spacer"></span>
    {#if !tab.running && summary}<span class="summary muted">{summary}</span>{/if}
    <RowLimitMenu value={tab.rowLimit} onchange={(limit) => (tab.rowLimit = limit)} />
  </div>

  {#if tab.running && shown?.status === 'running'}
    <div class="placeholder muted" role="status">
      <div class="spinner" aria-hidden="true"></div>
      <p>
        Running{tab.runs.length > 1 ? ` statement ${tab.shown + 1} of ${tab.runs.length}` : ''}…
        {duration(elapsed)}
      </p>
      <button class="btn" onclick={() => cancel(tab)}>Cancel</button>
    </div>
  {:else if shown?.error}
    <div class="error" role="alert">
      <h3>{ERROR_TITLES[shown.error.kind]}</h3>
      <p class="message">{shown.error.message}</p>
      {#if where}<p class="muted">
          Line {where.line}, column {where.column} of the statement.
        </p>{/if}
      {#if shown.error.detail}<p class="muted">{shown.error.detail}</p>{/if}
      {#if shown.error.hint}<p class="hint">{shown.error.hint}</p>{/if}
      {#if shown.error.code}<p class="code muted">Code {shown.error.code}</p>{/if}
    </div>
  {:else if result}
    {#if result.notices.length > 0}
      <ul class="notices">
        {#each result.notices as notice, i (i)}<li>{notice}</li>{/each}
      </ul>
    {/if}
    {#if result.raw !== null}
      <pre class="raw">{result.raw}</pre>
      {#if result.moreAvailable}<p class="more muted">Output cut off at 5 MB.</p>{/if}
    {:else if result.columns.length > 0}
      <Grid columns={result.columns} rows={result.rows} />
      {#if result.moreAvailable}
        <p class="more muted">
          Showing the first {count(result.rows.length)} rows; the query has more. Raise the limit to fetch
          more (paging arrives in M2).
        </p>
      {/if}
    {:else}
      <div class="placeholder muted">
        <Icon name="check" size={24} />
        <p>
          {result.rowsAffected !== null
            ? `Done. ${count(result.rowsAffected)} ${result.rowsAffected === 1 ? 'row' : 'rows'} affected.`
            : 'Done.'}
        </p>
      </div>
    {/if}
  {:else if shown?.status === 'skipped'}
    <div class="placeholder muted"><p>Not run: an earlier statement failed.</p></div>
  {:else}
    <div class="placeholder muted">
      <Icon name="database" size={28} />
      <p>
        {connections.byId(tab.connectionId)
          ? 'Results will appear here once you run a query.'
          : 'Choose a connection to run queries.'}
      </p>
    </div>
  {/if}
</div>

<style>
  .results {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    flex: none;
    border-bottom: 1px solid var(--border);
  }
  .section {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .spacer {
    flex: 1;
  }
  .summary {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .statements {
    display: flex;
    gap: 2px;
    overflow: hidden;
  }
  .statements button {
    height: 22px;
    padding: 0 8px;
    border: 1px solid transparent;
    border-radius: 4px;
    background: none;
    font-size: 12px;
    color: var(--text-muted);
  }
  .statements button:hover {
    background: var(--bg-hover);
  }
  .statements button.active {
    border-color: var(--border-strong);
    background: var(--bg);
    color: var(--text);
  }
  .statements button.failed {
    color: var(--danger);
  }
  .placeholder {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    min-height: 0;
    text-align: center;
  }
  .placeholder p {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }
  .spinner {
    width: 20px;
    height: 20px;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .error {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 16px 20px;
    user-select: text;
    -webkit-user-select: text;
  }
  .error h3 {
    margin: 0 0 6px;
    font-size: 13px;
    color: var(--danger);
  }
  .error p {
    margin: 0 0 6px;
  }
  .message {
    font-family: var(--editor-font);
    white-space: pre-wrap;
  }
  .hint {
    font-weight: 600;
  }
  .code {
    font-size: 12px;
  }
  .notices {
    margin: 0;
    padding: 6px 12px 6px 28px;
    flex: none;
    max-height: 96px;
    overflow: auto;
    font-size: 12px;
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
    user-select: text;
    -webkit-user-select: text;
  }
  .raw {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 10px 14px;
    overflow: auto;
    font: var(--grid-font-size) var(--grid-font);
    user-select: text;
    -webkit-user-select: text;
  }
  .more {
    flex: none;
    margin: 0;
    padding: 5px 12px;
    font-size: 12px;
    border-top: 1px solid var(--border);
    background: var(--bg-subtle);
  }
</style>
