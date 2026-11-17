import { describe, expect, it } from 'vitest';
import { applyToRows, buildRowEdits, editCount, stageEdit } from './editing';
import type { StatementRun } from './tabs.svelte';
import type { RunResult } from './types';

function statement(): StatementRun {
  const result = {
    columns: [],
    rows: [
      ['1', 'apple', '3'],
      ['2', 'pear', null],
    ],
    editable: { schema: 'public', table: 'fruit', columns: ['id', 'name', null], key: ['id'] },
  } as unknown as RunResult;
  return { sql: 'select', offset: 0, status: 'done', result };
}

describe('staged edits', () => {
  it('stages changes and counts them', () => {
    const s = statement();
    stageEdit(s, 0, 1, 'banana');
    stageEdit(s, 1, 1, null);
    expect(s.edits).toEqual({ 0: { 1: 'banana' }, 1: { 1: null } });
    expect(editCount(s.edits)).toBe(2);
  });

  it('setting a cell back to its fetched value removes the edit', () => {
    const s = statement();
    stageEdit(s, 0, 1, 'banana');
    stageEdit(s, 0, 1, 'apple');
    expect(s.edits).toEqual({});
    // NULL back to NULL is not an edit either.
    stageEdit(s, 1, 2, null);
    expect(editCount(s.edits)).toBe(0);
  });

  it('addresses each row by the key it was fetched with', () => {
    const s = statement();
    stageEdit(s, 0, 1, 'banana');
    stageEdit(s, 0, 0, '10'); // editing the key itself
    stageEdit(s, 1, 1, null);
    expect(buildRowEdits(s.result!, s.edits!)).toEqual([
      {
        key: ['1'],
        changes: [
          ['id', '10'],
          ['name', 'banana'],
        ],
      },
      { key: ['2'], changes: [['name', null]] },
    ]);
  });

  it('after saving, the rows show the new values and other rows are untouched', () => {
    const s = statement();
    stageEdit(s, 1, 1, 'plum');
    const rows = applyToRows(s.result!.rows, s.edits!);
    expect(rows[1]).toEqual(['2', 'plum', null]);
    expect(rows[0]).toBe(s.result!.rows[0]);
  });

  it('a result that is not editable produces no edits', () => {
    const s = statement();
    s.result!.editable = null;
    expect(buildRowEdits(s.result!, { 0: { 1: 'x' } })).toEqual([]);
  });
});
