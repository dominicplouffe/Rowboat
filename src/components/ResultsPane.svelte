<script lang="ts">
  import { ai } from '../lib/ai.svelte';
  import { looksLikeJson } from '../lib/cells';
  import { connections } from '../lib/connections.svelte';
  import { dialogs } from '../lib/dialogs.svelte';
  import { applyToRows, buildRowEdits, editCount, stageEdit } from '../lib/editing';
  import { asDbError, ipc } from '../lib/ipc';
  import { cancel, countRows, fetchAll, fetchNextPage, replaceRows } from '../lib/runner';
  import { settings } from '../lib/settings.svelte';
  import type { Tab } from '../lib/tabs.svelte';
  import type { DbError, ExportFormat } from '../lib/types';
  import Grid from './Grid.svelte';
  import Icon from './Icon.svelte';
  import Menu, { type MenuItem } from './Menu.svelte';
  import PlanTree from './PlanTree.svelte';
  import RowLimitMenu from './RowLimitMenu.svelte';
  import SidePanel from './SidePanel.svelte';

  let { tab }: { tab: Tab } = $props();

  const shown = $derived(tab.runs[tab.shown]);
  const result = $derived(shown?.result);
  const connection = $derived(connections.byId(tab.connectionId));

  // View state for the shown result. Reset when another result is shown.
  let filter = $state('');
  let hidden = $state<number[]>([]);
  let selected = $state<{ row: number; column: number } | null>(null);
  let recordView = $state(false);
  let jsonClosed = $state(false);
  let matching = $state(0);
  $effect(() => {
    void result?.columns;
    filter = '';
    hidden = [];
    recordView = false;
  });
  $effect(() => {
    void selected;
    jsonClosed = false;
  });

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
  const plural = (n: number, word: string) => `${count(n)} ${word}${n === 1 ? '' : 's'}`;

  const summary = $derived.by(() => {
    if (!result) return '';
    const parts: string[] = [];
    if (result.columns.length > 0) {
      parts.push(
        filter.trim()
          ? `${count(matching)} of ${plural(result.rows.length, 'row')}`
          : plural(result.rows.length, 'row'),
      );
      if (shown?.count !== undefined) parts.push(`${count(shown.count)} in total`);
      else if (result.moreAvailable) parts.push('more available');
    } else if (result.rowsAffected !== null) {
      parts.push(`${plural(result.rowsAffected, 'row')} affected`);
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

  // ---- side panel ----

  const selectedRow = $derived.by(() => {
    if (!result || !selected) return null;
    const row = result.rows[selected.row];
    if (!row) return null;
    const edits = shown?.edits?.[selected.row];
    return edits ? row.map((value, i) => (i in edits ? edits[i] : value)) : row;
  });
  const panel = $derived.by((): 'json' | 'record' | null => {
    if (!result || !selected || !selectedRow) return null;
    if (recordView) return 'record';
    const auto = settings.value?.results.jsonAutoDetect ?? true;
    const isJson = looksLikeJson(
      selectedRow[selected.column],
      result.columns[selected.column],
      auto,
    );
    return isJson && !jsonClosed ? 'json' : null;
  });

  // ---- columns, copy, export ----

  const tableName = $derived(
    result?.editable
      ? [result.editable.schema, result.editable.table].filter(Boolean).join('.')
      : 'table_name',
  );

  const columnItems = $derived<MenuItem[]>(
    (result?.columns ?? []).map((column, i) => ({
      label: column.name,
      checked: !hidden.includes(i),
      run: () => {
        hidden = hidden.includes(i) ? hidden.filter((c) => c !== i) : [...hidden, i];
      },
    })),
  );

  const FORMATS: [ExportFormat, string][] = [
    ['tsv', 'TSV'],
    ['csv', 'CSV'],
    ['json', 'JSON'],
    ['ndjson', 'NDJSON'],
    ['markdown', 'Markdown'],
    ['sql', 'SQL INSERT'],
  ];

  /** The loaded rows and visible columns, as they would be exported. */
  function page() {
    const keep = result!.columns.map((_, i) => i).filter((i) => !hidden.includes(i));
    return {
      columns: keep.map((i) => result!.columns[i]),
      rows: result!.rows.map((row) => keep.map((i) => row[i])),
    };
  }

  async function copyAs(format: ExportFormat) {
    if (!result || !connection || !shown) return;
    try {
      const text = await ipc.formatRows({
        format,
        kind: connection.kind,
        table: tableName,
        ...page(),
      });
      await navigator.clipboard.writeText(text);
      shown.note = `Copied ${plural(result.rows.length, 'row')}.`;
    } catch (e) {
      shown.note = `Could not copy: ${asDbError(e).message}`;
    }
  }

  async function exportPage(format: ExportFormat) {
    if (!result || !connection || !shown) return;
    try {
      const done = await ipc.exportPage({
        format,
        kind: connection.kind,
        table: tableName,
        ...page(),
      });
      if (done) shown.note = `Exported ${plural(done.rows, 'row')} to ${done.path}`;
    } catch (e) {
      shown.note = `Export failed: ${asDbError(e).message}`;
    }
  }

  async function exportAll(format: ExportFormat) {
    if (!connection || !shown || shown.busy) return;
    shown.busy = 'export';
    tab.queryId = crypto.randomUUID();
    try {
      const done = await ipc.exportQuery({
        connectionId: connection.id,
        queryId: tab.queryId,
        sql: shown.sql,
        format,
        table: tableName,
      });
      if (done) shown.note = `Exported ${plural(done.rows, 'row')} to ${done.path}`;
    } catch (e) {
      const error = asDbError(e);
      shown.note =
        error.kind === 'cancelled' ? 'Export cancelled.' : `Export failed: ${error.message}`;
    } finally {
      shown.busy = undefined;
      tab.queryId = null;
    }
  }

  const copyItems = $derived<MenuItem[]>(
    FORMATS.map(([format, label]) => ({ label: `Copy as ${label}`, run: () => copyAs(format) })),
  );
  const exportItems = $derived<MenuItem[]>([
    ...FORMATS.map(([format, label]) => ({
      label: `Loaded rows as ${label}`,
      run: () => exportPage(format),
    })),
    // The full result is fetched by running the statement again, which is
    // only offered for plain reads.
    ...(result?.repeatable
      ? FORMATS.map(([format, label], i) => ({
          label: `Full result as ${label}`,
          separator: i === 0,
          run: () => exportAll(format),
        }))
      : []),
  ]);

  // ---- inline editing ----

  const pending = $derived(editCount(shown?.edits));

  async function previewEdits() {
    if (!result || !shown?.edits || !connection) return;
    try {
      const statements = await ipc.editsPreview(
        connection.id,
        result.editable!,
        buildRowEdits(result, shown.edits),
      );
      const apply = await dialogs.confirm({
        title: `Apply ${plural(statements.length, 'update')} to ${connection.name}?`,
        message: 'These statements run in one transaction: either all of them are saved or none.',
        detail: statements.map((s) => `${s};`).join('\n'),
        confirmLabel: 'Apply',
        prod: connection.environment === 'prod',
      });
      if (apply) await applyEdits();
    } catch (e) {
      shown.note = asDbError(e).message;
    }
  }

  async function applyEdits() {
    if (!result || !shown?.edits || !connection || shown.busy) return;
    shown.busy = 'apply';
    try {
      const saved = await ipc.editsApply(
        connection.id,
        crypto.randomUUID(),
        result.editable!,
        buildRowEdits(result, shown.edits),
      );
      replaceRows(shown, applyToRows(result.rows, shown.edits));
      shown.edits = {};
      shown.note = `Saved ${plural(saved, 'row')}.`;
    } catch (e) {
      const error = asDbError(e);
      shown.note = [error.message, error.detail].filter(Boolean).join(' — ');
    } finally {
      shown.busy = undefined;
    }
  }

  async function fetchEverything() {
    await fetchAll(tab);
    const now = tab.runs[tab.shown];
    if (!now?.result?.stoppedAtMemoryCap) return;
    const cap = settings.value?.execution.resultMemoryCapMb ?? 256;
    const more = await dialogs.confirm({
      title: 'This result is larger than the memory cap',
      message: `Fetching stopped at ${cap} MB (${plural(now.result.rows.length, 'row')}). Fetching the rest may use a lot of memory. To save everything without loading it, use Export → Full result instead.`,
      confirmLabel: 'Fetch all anyway',
      danger: true,
    });
    if (more) await fetchAll(tab, true);
  }

  const BUSY_TEXT = {
    page: 'Fetching next page…',
    all: 'Fetching all rows…',
    count: 'Counting rows…',
    export: 'Exporting…',
    apply: 'Saving edits…',
  } as const;
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
    {#if result && result.columns.length > 0}
      <label class="filter">
        <Icon name="search" size={13} />
        <input
          type="search"
          placeholder="Filter rows"
          aria-label="Filter loaded rows"
          spellcheck="false"
          bind:value={filter}
        />
      </label>
    {/if}
    <span class="spacer"></span>
    {#if !tab.running && summary}<span class="summary muted">{summary}</span>{/if}
    {#if result && result.columns.length > 0}
      <Menu label="Columns" items={columnItems} title="Show or hide columns" />
      <Menu label="Copy" items={copyItems} title="Copy the loaded rows" />
      <Menu
        label="Export"
        items={exportItems}
        disabled={!!shown?.busy}
        title="Save rows to a file"
      />
      <button
        class="btn"
        aria-pressed={recordView}
        class:on={recordView}
        title="Show the selected row as a form"
        onclick={() => (recordView = !recordView)}
      >
        Record
      </button>
    {/if}
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
      {#if shown.error.code && shown.error.code !== 'connection-closed'}<p class="code muted">
          Code {shown.error.code}
        </p>{/if}
      {#if ai.fixable(tab, shown)}
        {#if shown.fix && shown.fix.status !== 'dismissed'}
          {#await import('./FixSuggestion.svelte') then { default: FixSuggestion }}
            <FixSuggestion {tab} statement={shown} />
          {/await}
        {:else}
          <button class="btn" onclick={() => ai.suggestFix(tab, shown)}
            >Suggest a fix with AI</button
          >
        {/if}
      {/if}
    </div>
  {:else if result}
    {#if result.notices.length > 0}
      <ul class="notices">
        {#each result.notices as notice, i (i)}<li>{notice}</li>{/each}
      </ul>
    {/if}
    {#if shown?.plan && connection && result.columns.length > 0}
      <PlanTree kind={connection.kind} {result} />
    {:else if result.raw !== null}
      <pre class="raw">{result.raw}</pre>
      {#if result.moreAvailable}<p class="status muted">Output cut off at 5 MB.</p>{/if}
    {:else if result.columns.length > 0}
      <div class="data">
        <Grid
          columns={result.columns}
          rows={result.rows}
          {filter}
          {hidden}
          editable={connection?.readOnly ? null : result.editable}
          edits={shown?.edits ?? {}}
          bind:selected
          onedit={(row, column, value) => shown && stageEdit(shown, row, column, value)}
          onshown={(n) => (matching = n)}
        />
        {#if panel && selected && selectedRow}
          <SidePanel
            mode={panel}
            columns={result.columns}
            row={selectedRow}
            column={selected.column}
            onclose={() => (panel === 'record' ? (recordView = false) : (jsonClosed = true))}
          />
        {/if}
      </div>

      {#if pending > 0}
        <div class="status edits">
          <strong>{plural(pending, 'unsaved edit')}</strong>
          <span class="spacer"></span>
          <button class="btn" disabled={!!shown?.busy} onclick={previewEdits}>Preview SQL…</button>
          <button class="btn primary" disabled={!!shown?.busy} onclick={applyEdits}>Apply</button>
          <button class="btn" disabled={!!shown?.busy} onclick={() => shown && (shown.edits = {})}
            >Discard</button
          >
        </div>
      {/if}

      <div class="status">
        {#if shown?.busy}
          <span role="status">{BUSY_TEXT[shown.busy]}</span>
          {#if shown.busy !== 'apply'}<button class="btn" onclick={() => cancel(tab)}>Cancel</button
            >{/if}
        {:else}
          {#if result.moreAvailable && !result.repeatable}
            <span class="muted">
              More rows exist, but this statement changes data, so it is not run again to fetch
              them.
            </span>
          {:else if result.moreAvailable}
            <span class="muted">More rows available.</span>
            <button class="btn" onclick={() => fetchNextPage(tab)}>Fetch next page</button>
            <button class="btn" onclick={fetchEverything}>Fetch all</button>
          {:else}
            <span class="muted">All rows loaded.</span>
          {/if}
          {#if shown?.count === undefined && result.repeatable}
            <button class="btn" onclick={() => countRows(tab)}>Count rows</button>
          {/if}
          {#if result.editable && !connection?.readOnly}
            <span class="muted hint-edit">Double-click a cell to edit.</span>
          {/if}
        {/if}
        {#if shown?.note}<span class="note" role="status" title={shown.note}>{shown.note}</span
          >{/if}
      </div>
    {:else}
      <div class="placeholder muted">
        <Icon name="check" size={24} />
        <p>
          {result.rowsAffected !== null
            ? `Done. ${plural(result.rowsAffected, 'row')} affected.`
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
        {connection
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
    gap: 6px;
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
    white-space: nowrap;
  }
  .filter {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--row);
    width: 180px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    color: var(--text-muted);
  }
  .filter:focus-within {
    outline: 2px solid var(--focus);
    outline-offset: -1px;
  }
  .filter input {
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
  .btn.on {
    background: var(--bg-active);
    border-color: var(--focus);
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
  .data {
    display: flex;
    flex: 1;
    min-height: 0;
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
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
    min-height: 34px;
    margin: 0;
    padding: 4px 10px;
    font-size: 12px;
    border-top: 1px solid var(--border);
    background: var(--bg-subtle);
  }
  .status .btn {
    height: 24px;
  }
  .status.edits {
    background: rgba(232, 163, 61, 0.16);
  }
  .note {
    margin-left: auto;
    max-width: 55%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    user-select: text;
    -webkit-user-select: text;
  }
  .hint-edit {
    margin-left: 4px;
  }
</style>
