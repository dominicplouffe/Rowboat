// Runs SQL from a tab: which statements, in what order, and what happens on
// errors. The statements are split by the Rust core and sent one at a time,
// exactly as written.

import { connections } from './connections.svelte';
import { asDbError, ipc } from './ipc';
import { settings } from './settings.svelte';
import type { StatementRun, Tab } from './tabs.svelte';
import type { StatementSpan } from './types';

export type RunScope = 'statement' | 'script';

export interface EditorSelection {
  /** Selection start and end (equal when nothing is selected), UTF-16 offsets. */
  from: number;
  to: number;
}

/** Asks the user to confirm a destructive statement. Set by the UI. */
export const prompts: {
  confirmDestructive: (sql: string, reason: string, connectionName: string) => Promise<boolean>;
} = {
  // Until the UI installs its dialog, nothing destructive is confirmed.
  confirmDestructive: async () => false,
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
          statement.result = await ipc.queryRun({
            connectionId: connection.id,
            queryId: tab.queryId,
            sql: statement.sql,
            rowLimit: tab.rowLimit,
            confirmed,
          });
          statement.status = 'done';
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
          if (error.kind === 'connection') connections.markClosed(connection.id);
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
