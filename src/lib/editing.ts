// Staged inline edits: kept per result until the user applies or discards them.

import type { StatementRun } from './tabs.svelte';
import type { RowEdit, RunResult } from './types';

type Edits = NonNullable<StatementRun['edits']>;

/** Stages a new value for a cell; setting it back to the fetched value unstages it. */
export function stageEdit(
  statement: StatementRun,
  row: number,
  column: number,
  value: string | null,
): void {
  const original = statement.result?.rows[row]?.[column];
  if (original === undefined) return;
  const edits = (statement.edits ??= {});
  if (value === original) {
    if (edits[row]) {
      delete edits[row][column];
      if (Object.keys(edits[row]).length === 0) delete edits[row];
    }
    return;
  }
  (edits[row] ??= {})[column] = value;
}

export function editCount(edits: Edits | undefined): number {
  return Object.values(edits ?? {}).reduce((total, row) => total + Object.keys(row).length, 0);
}

/** The edits in the form the core applies: each row addressed by its primary key. */
export function buildRowEdits(result: RunResult, edits: Edits): RowEdit[] {
  const target = result.editable;
  if (!target) return [];
  const keyColumns = target.key.map((name) => target.columns.indexOf(name));
  return Object.entries(edits)
    .map(([row, cells]) => ({
      // The key as it was fetched, even if a key column itself is being edited.
      key: keyColumns.map((column) => result.rows[Number(row)][column]),
      changes: Object.entries(cells).map(
        ([column, value]) => [target.columns[Number(column)]!, value] as [string, string | null],
      ),
    }))
    .filter((edit) => edit.changes.length > 0);
}

/** The rows as they are after the edits were saved. */
export function applyToRows(rows: (string | null)[][], edits: Edits): (string | null)[][] {
  return rows.map((row, index) => {
    const cells = edits[index];
    if (!cells) return row;
    const copy = [...row];
    for (const [column, value] of Object.entries(cells)) copy[Number(column)] = value;
    return copy;
  });
}
