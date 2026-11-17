import { beforeEach, describe, expect, it } from 'vitest';
import { blankConnection, connections } from './connections.svelte';
import { errorOffset, prompts, run, statementAtCursor } from './runner';
import { settings } from './settings.svelte';
import { tabs, type Tab } from './tabs.svelte';

const cursor = (at: number) => ({ from: at, to: at });

describe('statementAtCursor', () => {
  // "select 1; select 2;  select 3"
  const spans = [
    { start: 0, end: 8 },
    { start: 10, end: 18 },
    { start: 21, end: 29 },
  ];

  it('picks the statement the cursor is inside or touching', () => {
    expect(statementAtCursor(spans, 3)).toEqual([spans[0]]);
    expect(statementAtCursor(spans, 8)).toEqual([spans[0]]);
    expect(statementAtCursor(spans, 10)).toEqual([spans[1]]);
    expect(statementAtCursor(spans, 29)).toEqual([spans[2]]);
  });

  it('between statements, picks the one before the cursor', () => {
    expect(statementAtCursor(spans, 9)).toEqual([spans[0]]);
    expect(statementAtCursor(spans, 20)).toEqual([spans[1]]);
  });

  it('before everything, picks the first; with nothing, picks nothing', () => {
    expect(statementAtCursor([{ start: 5, end: 9 }], 0)).toEqual([{ start: 5, end: 9 }]);
    expect(statementAtCursor([], 0)).toEqual([]);
  });
});

describe('running statements', () => {
  let tab: Tab;

  beforeEach(async () => {
    await settings.load();
    for (const t of [...tabs.list]) tabs.close(t.id);
    for (const c of [...connections.list]) await connections.remove(c.id);
    const saved = await connections.save({ ...blankConnection(), name: 'dev' }, {});
    tab = tabs.open(1000, '', { connectionId: saved.id });
    prompts.confirmDestructive = async () => false;
  });

  it('connects on first run and runs only the statement under the cursor', async () => {
    tab.text = 'select 1; select 2';
    await run(tab, cursor(12), 'statement');
    expect(connections.byId(tab.connectionId)?.connected).toBe(true);
    expect(tab.runs.map((r) => r.sql)).toEqual(['select 2']);
    expect(tab.runs[0].status).toBe('done');
    expect(tab.runs[0].offset).toBe(10);
    expect(tab.running).toBe(false);
  });

  it('a selection runs exactly the selected statements', async () => {
    tab.text = 'select 1; select 2; select 3';
    await run(tab, { from: 10, to: 28 }, 'statement');
    expect(tab.runs.map((r) => r.sql)).toEqual(['select 2', 'select 3']);
    expect(tab.runs.map((r) => r.offset)).toEqual([10, 20]);
  });

  it('run script runs everything and shows the last result', async () => {
    tab.text = 'select 1; update t set a = 1; select 3';
    await run(tab, cursor(0), 'script');
    expect(tab.runs.map((r) => r.status)).toEqual(['done', 'done', 'done']);
    expect(tab.shown).toBe(2);
    expect(tab.runs[1].result?.rowsAffected).toBe(1);
  });

  it('the row limit of the tab is what gets fetched', async () => {
    tab.text = 'select * from big';
    tab.rowLimit = 100;
    await run(tab, cursor(0), 'statement');
    expect(tab.runs[0].result?.rows).toHaveLength(100);
    expect(tab.runs[0].result?.moreAvailable).toBe(true);
  });

  it('stops a script at the first error and skips the rest', async () => {
    tab.text = 'select 1; select * fromm t; select 3';
    await run(tab, cursor(0), 'script');
    expect(tab.runs.map((r) => r.status)).toEqual(['done', 'failed', 'skipped']);
    expect(tab.shown).toBe(1);
    expect(tab.runs[1].error?.kind).toBe('syntax');
  });

  it('continues past errors when the setting says so', async () => {
    settings.set('execution', 'stopScriptOnError', false);
    tab.text = 'select * fromm t; select 3';
    await run(tab, cursor(0), 'script');
    expect(tab.runs.map((r) => r.status)).toEqual(['failed', 'done']);
    settings.set('execution', 'stopScriptOnError', true);
  });

  it('maps the database error position back into the editor text', async () => {
    tab.text = 'select 1;\nselect * fromm t';
    await run(tab, cursor(0), 'script');
    // "fromm" starts at offset 19 of the whole text.
    expect(errorOffset(tab.runs[1])).toBe(tab.text.indexOf('fromm'));
  });

  it('error positions count characters, the editor counts UTF-16 units', () => {
    const statement = {
      sql: "select '😀' fromm t",
      offset: 100,
      status: 'failed' as const,
      error: {
        kind: 'syntax' as const,
        message: '',
        detail: null,
        hint: null,
        code: null,
        position: 12,
      },
    };
    // 11 characters precede the error; the emoji is two UTF-16 units.
    expect(errorOffset(statement)).toBe(100 + 12);
    expect(errorOffset({ ...statement, error: { ...statement.error, position: null } })).toBeNull();
  });

  it('a destructive statement on Prod runs only after the user confirms', async () => {
    const prod = await connections.save(
      { ...blankConnection(), name: 'prod', environment: 'prod' },
      {},
    );
    tab.connectionId = prod.id;
    tab.text = 'drop table users';

    const asked: string[] = [];
    prompts.confirmDestructive = async (sql, reason, name) => {
      asked.push(`${name}: ${sql} (${reason})`);
      return false;
    };
    await run(tab, cursor(0), 'statement');
    expect(asked).toEqual(['prod: drop table users (DROP removes objects permanently.)']);
    expect(tab.runs[0].status).toBe('failed');
    expect(tab.runs[0].error?.kind).toBe('cancelled');

    prompts.confirmDestructive = async () => true;
    await run(tab, cursor(0), 'statement');
    expect(tab.runs[0].status).toBe('done');
  });

  it('a failed connection shows its error instead of running', async () => {
    const bad = await connections.save(
      { ...blankConnection(), name: 'down', host: 'unreachable' },
      {},
    );
    tab.connectionId = bad.id;
    tab.text = 'select 1';
    await run(tab, cursor(0), 'statement');
    expect(tab.runs[0].error?.kind).toBe('connection');
    expect(connections.byId(bad.id)?.connected).toBe(false);
  });

  it('after the connection is lost, the next run opens a fresh one', async () => {
    tab.text = 'select lost_connection';
    await run(tab, cursor(0), 'statement');
    expect(tab.runs[0].error?.kind).toBe('connection');
    expect(connections.byId(tab.connectionId)?.connected).toBe(false);

    tab.text = 'select 1';
    await run(tab, cursor(0), 'statement');
    expect(tab.runs[0].status).toBe('done');
    expect(connections.byId(tab.connectionId)?.connected).toBe(true);
  });

  it('auto-commit is switched per connection, not globally', async () => {
    const other = await connections.save({ ...blankConnection(), name: 'other' }, {});
    await connections.setAutoCommit(tab.connectionId!, false);
    await connections.load();
    expect(connections.byId(tab.connectionId)?.autoCommit).toBe(false);
    expect(connections.byId(other.id)?.autoCommit).toBe(true);
    expect(settings.value?.execution.autoCommit).toBe(true);
    await connections.setAutoCommit(tab.connectionId!, true);
  });

  it('result rows stay plain arrays instead of reactive proxies', async () => {
    tab.text = 'select * from big';
    await run(tab, cursor(0), 'statement');
    const rows = tab.runs[0].result!.rows;
    expect(rows[0][0]).toBe('1');
    // Reading through a Svelte proxy returns a wrapper; a plain array is itself.
    expect(tab.runs[0].result!.rows).toBe(rows);
    expect(Object.getPrototypeOf(tab.runs[0].result)).not.toBe(Object.prototype);
  });

  it('generated (DDL) tabs never run', async () => {
    tab.generated = true;
    tab.text = 'drop table users';
    await run(tab, cursor(0), 'script');
    expect(tab.runs).toEqual([]);
  });
});
