// Runs SQL from a tab: which statements, in what order, and what happens on
// errors. The statements are split by the Rust core and sent one at a time,
// exactly as written.

import { connections } from './connections.svelte';
import { asDbError, ipc } from './ipc';
import { schemaCache, changesSchema } from './schemaCache.svelte';
import { settings } from './settings.svelte';
import type { StatementRun, Tab } from './tabs.svelte';
import type { DbKind, RunResult, StatementSpan } from './types';

export type RunScope = 'statement' | 'script';

export interface EditorSelection {
  /** Selection start and end (equal when nothing is selected), UTF-16 offsets. */
  from: number;
  to: number;
}

/** Asks the user to confirm a destructive statement. Set by the UI. */
export const prompts: {
  confirmDestructive: (sql: string, reason: string, connectionName: string) => Promise<boolean>;
  /** Told when a statement changed the schema; the copilot re-indexes it. */
  schemaChanged: (connectionId: string) => void;
  /** Told about every failed statement; the copilot may suggest a fix. */
  failed: (tab: Tab, statement: StatementRun) => void;
} = {
  // Until the UI installs its dialog, nothing destructive is confirmed.
  confirmDestructive: async () => false,
  schemaChanged: () => {},
  failed: () => {},
};

/**
 * The statement the cursor is in: the one containing or touching it, else the
 * one just before it, else the first one.
 */
export function statementAtCursor(spans: StatementSpan[], cursor: number): StatementSpan[] {
  const containing = spans.find((s) => cursor >= s.start && cursor <= s.end);
  if (containing) return [containing];
  const before = [...spans].reverse().find((s) => s.end <= cursor);
  const chosen = before ?? spans[0];
  return chosen ? [chosen] : [];
}

/**
 * Results can hold hundreds of thousands of rows. Svelte makes plain objects
 * and arrays stored in `$state` deeply reactive by wrapping them in proxies,
 * which would put a proxy between the grid and every cell. An object with its
 * own class is stored as is, so the rows stay plain arrays.
 */
class Result {}
function unproxied(result: RunResult): RunResult {
  return Object.assign(new Result(), result) as RunResult;
}

function newQueryId(): string {
  return crypto.randomUUID();
}

/** Runs statements from `tab`. Resolves when all have finished or one stopped the run. */
export async function run(tab: Tab, selection: EditorSelection, scope: RunScope): Promise<void> {
  if (tab.running || tab.generated) return;
  const connection = connections.byId(tab.connectionId);
  if (!connection) return;

  tab.running = true;
  try {
    if (!connection.connected && !(await connections.open(connection.id))) {
      tab.runs = [
        { sql: '', offset: 0, status: 'failed', error: connections.errors[connection.id] },
      ];
      tab.shown = 0;
      return;
    }

    const selected = scope === 'statement' && selection.from !== selection.to;
    const source = selected ? tab.text.slice(selection.from, selection.to) : tab.text;
    const base = selected ? selection.from : 0;
    const spans = await ipc.sqlSplit(connection.kind, source);
    // A selection or "run script" runs every statement in the text; otherwise
    // only the statement under the cursor runs.
    const chosen =
      selected || scope === 'script' ? spans : statementAtCursor(spans, selection.from);
    if (chosen.length === 0) return;

    tab.runs = chosen.map((span): StatementRun => ({
      sql: source.slice(span.start, span.end),
      offset: base + span.start,
      status: 'pending',
    }));
    tab.shown = 0;
    const stopOnError = settings.value?.execution.stopScriptOnError ?? true;

    for (let i = 0; i < tab.runs.length; i++) {
      const statement = tab.runs[i];
      statement.status = 'running';
      tab.shown = i;
      let confirmed = false;
      for (;;) {
        tab.queryId = newQueryId();
        try {
          statement.result = unproxied(
            await ipc.queryRun({
              connectionId: connection.id,
              queryId: tab.queryId,
              sql: statement.sql,
              options: { rowLimit: tab.rowLimit, confirmed },
            }),
          );
          statement.status = 'done';
          // Autocomplete must know about objects this statement created or changed.
          if (changesSchema(statement.sql)) {
            void schemaCache.refresh(connection.id, connection.kind);
            prompts.schemaChanged(connection.id);
          }
          connections.setInTransaction(connection.id, statement.result.inTransaction);
        } catch (e) {
          const error = asDbError(e);
          if (error.kind === 'needs-confirmation' && !confirmed) {
            if (await prompts.confirmDestructive(statement.sql, error.message, connection.name)) {
              confirmed = true;
              continue;
            }
            error.kind = 'cancelled';
            error.message = 'Not run: you chose not to confirm this statement.';
          }
          statement.error = error;
          statement.status = 'failed';
          if (error.kind === 'connection') await connections.dropDead(connection.id);
          prompts.failed(tab, statement);
        }
        break;
      }
      if (statement.status === 'failed' && (stopOnError || statement.error?.kind === 'cancelled')) {
        for (const rest of tab.runs.slice(i + 1)) rest.status = 'skipped';
        break;
      }
    }
    // Show the first failure if there was one, otherwise the last result.
    const failed = tab.runs.findIndex((r) => r.status === 'failed');
    tab.shown = failed >= 0 ? failed : tab.runs.length - 1;
  } finally {
    tab.running = false;
    tab.queryId = null;
  }
}

export async function cancel(tab: Tab): Promise<void> {
  if (tab.queryId) await ipc.queryCancel(tab.queryId);
}

export async function endTransaction(connectionId: string, commit: boolean): Promise<void> {
  await ipc.transactionEnd(connectionId, commit);
  connections.setInTransaction(connectionId, false);
}

/**
 * Converts a database error position (1-based, in characters of the statement)
 * to a UTF-16 offset in the editor text.
 */
export function errorOffset(statement: StatementRun): number | null {
  const position = statement.error?.position;
  if (!position || position < 1) return null;
  const chars = [...statement.sql].slice(0, position - 1);
  return statement.offset + chars.join('').length;
}

// ---- Follow-ups on a result: more rows, all rows, a count ----

/** Rows added by "Fetch next page": the tab's limit, or 1,000 when it has none. */
function pageSize(tab: Tab): number {
  return tab.rowLimit || 1000;
}

async function refetch(
  tab: Tab,
  statement: StatementRun,
  options: { rowLimit: number; uncapped?: boolean },
): Promise<void> {
  const connection = connections.byId(tab.connectionId);
  if (!connection || !statement.result) return;
  tab.queryId = newQueryId();
  try {
    // The statement runs again with a larger fetch window; rows already on
    // screen are fetched again, which keeps no cursor or transaction open on
    // the server between pages.
    const result = await ipc.queryRun({
      connectionId: connection.id,
      queryId: tab.queryId,
      sql: statement.sql,
      // The user confirmed this statement when it first ran. `refetch` makes
      // the core refuse anything that is not a plain read.
      options: { ...options, confirmed: true, refetch: true },
    });
    statement.result = unproxied(result);
    // Staged edits are addressed by row position, which a new fetch may change.
    statement.edits = undefined;
    statement.note = result.stoppedAtMemoryCap
      ? `Stopped at the result memory cap after ${result.rows.length.toLocaleString('en-US')} rows.`
      : undefined;
  } catch (e) {
    const error = asDbError(e);
    statement.note = error.kind === 'cancelled' ? 'Cancelled.' : error.message;
    if (error.kind === 'connection') await connections.dropDead(connection.id);
  } finally {
    tab.queryId = null;
  }
}

/** Fetches one more page of the shown result. */
/**
 * Whether the shown result may be fetched again. A statement that changes
 * data is never repeated, and neither is one with edits waiting: they are
 * addressed by row position, which a new fetch could change.
 */
function mayRefetch(statement: StatementRun | undefined): statement is StatementRun {
  if (!statement?.result || !statement.result.repeatable) return false;
  if (Object.keys(statement.edits ?? {}).length > 0) {
    statement.note = 'Apply or discard your edits before fetching more rows.';
    return false;
  }
  return true;
}

export async function fetchNextPage(tab: Tab): Promise<void> {
  const statement = tab.runs[tab.shown];
  if (!mayRefetch(statement) || !statement.result || statement.busy || tab.running) return;
  statement.busy = 'page';
  try {
    await refetch(tab, statement, { rowLimit: statement.result.rows.length + pageSize(tab) });
  } finally {
    statement.busy = undefined;
  }
}

/** Fetches every row; `uncapped` goes past the result memory cap. */
export async function fetchAll(tab: Tab, uncapped = false): Promise<void> {
  const statement = tab.runs[tab.shown];
  if (!mayRefetch(statement) || !statement.result || statement.busy || tab.running) return;
  statement.busy = 'all';
  try {
    await refetch(tab, statement, { rowLimit: 0, uncapped });
  } finally {
    statement.busy = undefined;
  }
}

/** Asks the database how many rows the shown query returns. */
export async function countRows(tab: Tab): Promise<void> {
  const statement = tab.runs[tab.shown];
  const connection = connections.byId(tab.connectionId);
  if (!statement?.result?.repeatable || !connection || statement.busy || tab.running) return;
  statement.busy = 'count';
  tab.queryId = newQueryId();
  try {
    statement.count = await ipc.queryCount(connection.id, tab.queryId, statement.sql);
    statement.note = undefined;
  } catch (e) {
    const error = asDbError(e);
    statement.note = [error.message, error.hint].filter(Boolean).join(' ');
  } finally {
    statement.busy = undefined;
    tab.queryId = null;
  }
}

/** Replaces the shown result's rows (after edits were applied). */
export function replaceRows(statement: StatementRun, rows: (string | null)[][]): void {
  if (statement.result) statement.result = unproxied({ ...statement.result, rows });
}

/** The EXPLAIN form of a statement for each database. */
export function explainSql(kind: DbKind, sql: string): string {
  const body = sql.trim().replace(/;+\s*$/, '');
  if (kind === 'postgres') return `EXPLAIN (FORMAT JSON) ${body}`;
  if (kind === 'sqlite') return `EXPLAIN QUERY PLAN ${body}`;
  return `EXPLAIN ${body}`;
}

/** Shows the plan of the statement under the cursor, without running it. */
export async function explain(tab: Tab, selection: EditorSelection): Promise<void> {
  if (tab.running || tab.generated) return;
  const connection = connections.byId(tab.connectionId);
  if (!connection) return;
  tab.running = true;
  try {
    if (!connection.connected && !(await connections.open(connection.id))) return;
    const selected = selection.from !== selection.to;
    const source = selected ? tab.text.slice(selection.from, selection.to) : tab.text;
    const spans = await ipc.sqlSplit(connection.kind, source);
    const [span] = selected ? spans : statementAtCursor(spans, selection.from);
    if (!span) return;
    const statement: StatementRun = {
      sql: explainSql(connection.kind, source.slice(span.start, span.end)),
      offset: 0,
      status: 'running',
      plan: true,
    };
    tab.runs = [statement];
    tab.shown = 0;
    const live = tab.runs[0];
    tab.queryId = newQueryId();
    try {
      live.result = unproxied(
        await ipc.queryRun({
          connectionId: connection.id,
          queryId: tab.queryId,
          sql: live.sql,
          options: { rowLimit: 0, confirmed: true },
        }),
      );
      live.status = 'done';
    } catch (e) {
      live.error = asDbError(e);
      // Positions refer to the EXPLAIN text, not to the editor.
      live.error.position = null;
      live.status = 'failed';
    }
  } finally {
    tab.running = false;
    tab.queryId = null;
  }
}
