import { beforeEach, describe, expect, it } from 'vitest';
import { ipc } from './ipc';
import { loadSaved, parse, restore, serialize } from './session';
import { tabs } from './tabs.svelte';

describe('session restore', () => {
  beforeEach(() => {
    for (const tab of [...tabs.list]) tabs.close(tab.id);
  });

  it('saves what the user typed, not results or generated tabs', () => {
    const a = tabs.open(1000, 'select 1', { connectionId: 'c1' });
    tabs.open(500, 'CREATE TABLE …', { generated: true });
    const b = tabs.open(0, 'select 2', { path: '/tmp/q.sql', title: 'q.sql' });
    a.runs = [{ sql: 'select 1', offset: 0, status: 'done' }];

    const saved = serialize(tabs.list, b.id);
    expect(saved).toEqual({
      tabs: [
        { title: a.title, text: 'select 1', connectionId: 'c1', rowLimit: 1000, path: null },
        { title: 'q.sql', text: 'select 2', connectionId: 'c1', rowLimit: 0, path: '/tmp/q.sql' },
      ],
      active: 1,
    });
  });

  it('round-trips through the core, reopening tabs with their text and selection', async () => {
    tabs.open(1000, 'select 1', { connectionId: 'known' });
    const second = tabs.open(250, 'unsaved work', { connectionId: 'deleted-since' });
    tabs.activeId = second.id;
    await ipc.sessionSave(JSON.stringify(serialize(tabs.list, tabs.activeId)));
    for (const tab of [...tabs.list]) tabs.close(tab.id);

    expect(restore(new Set(['known']), await loadSaved())).toBe(true);
    expect(tabs.list.map((t) => t.text)).toEqual(['select 1', 'unsaved work']);
    expect(tabs.active?.text).toBe('unsaved work');
    expect(tabs.active?.rowLimit).toBe(250);
    // The connection that no longer exists is not selected.
    expect(tabs.list.map((t) => t.connectionId)).toEqual(['known', null]);
  });

  it('ignores a missing or damaged session', () => {
    expect(parse(null)).toBeNull();
    expect(parse('not json')).toBeNull();
    expect(parse('{"tabs": []}')).toBeNull();
    expect(parse('{"tabs": "nope"}')).toBeNull();
    expect(parse('{"tabs":[{"text":"ok","rowLimit":-5}],"active":9}')).toEqual({
      tabs: [{ title: 'Query', text: 'ok', connectionId: null, rowLimit: 1000, path: null }],
      active: 0,
    });
  });
});
