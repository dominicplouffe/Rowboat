import { beforeEach, describe, expect, it } from 'vitest';
import { ipc } from './ipc';
import { settings } from './settings.svelte';
import { tabs } from './tabs.svelte';
import type { Settings } from './types';
import { ui } from './ui.svelte';

describe('tabs', () => {
  beforeEach(() => {
    for (const tab of [...tabs.list]) tabs.close(tab.id);
  });

  it('opens tabs with their own row limit and activates them', () => {
    const a = tabs.open(1000);
    const b = tabs.open(500, 'select 1');
    expect(tabs.active?.id).toBe(b.id);
    expect(a.rowLimit).toBe(1000);
    expect(b.text).toBe('select 1');
  });

  it('changing one tab’s limit leaves the others alone', () => {
    const a = tabs.open(1000);
    const b = tabs.open(1000);
    b.rowLimit = 5000;
    expect(tabs.list.find((t) => t.id === a.id)?.rowLimit).toBe(1000);
  });

  it('closing the active tab selects its neighbour', () => {
    const a = tabs.open(1000);
    const b = tabs.open(1000);
    const c = tabs.open(1000);
    tabs.activeId = b.id;
    tabs.close(b.id);
    expect(tabs.activeId).toBe(c.id);
    tabs.close(c.id);
    expect(tabs.activeId).toBe(a.id);
    tabs.close(a.id);
    expect(tabs.active).toBeUndefined();
  });

  it('cycles through tabs in both directions', () => {
    const a = tabs.open(1000);
    const b = tabs.open(1000);
    tabs.cycle(1);
    expect(tabs.activeId).toBe(a.id);
    tabs.cycle(-1);
    expect(tabs.activeId).toBe(b.id);
  });
});

describe('settings store', () => {
  beforeEach(async () => {
    await settings.load();
  });

  it('loads settings from the core', () => {
    expect(settings.value?.execution.defaultRowLimit).toBe(1000);
  });

  it('persists a change to the core', async () => {
    settings.set('execution', 'defaultRowLimit', 500);
    await new Promise((r) => setTimeout(r, 0));
    expect((await ipc.settingsGet()).execution.defaultRowLimit).toBe(500);
    settings.set('execution', 'defaultRowLimit', 1000);
  });

  it('applies the theme to the document', () => {
    settings.set('appearance', 'theme', 'dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
    settings.set('appearance', 'theme', 'system');
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });

  it('stores only rebound shortcuts', async () => {
    settings.setShortcut('settings.open', 'Mod+Shift+P');
    expect(settings.value?.keyboard.overrides).toEqual({ 'settings.open': 'Mod+Shift+P' });
    settings.setShortcut('settings.open', null);
    expect(settings.value?.keyboard.overrides).toEqual({});
  });
});

describe('update check', () => {
  /** Stores a change and waits for the core to have it. */
  async function configure(change: Partial<Settings['general']>): Promise<void> {
    const current = await ipc.settingsGet();
    await ipc.settingsSet({ ...current, general: { ...current.general, ...change } });
    await settings.load();
  }
  const found = () => ui.update;

  beforeEach(async () => {
    await ipc.settingsReset();
    ui.update = null;
  });

  it('finds nothing on the stable channel and a pre-release on beta', async () => {
    await configure({ updateChannel: 'stable' });
    expect(await ui.checkForUpdate(false)).toBeNull();
    expect(found()).toBeNull();

    await configure({ updateChannel: 'beta' });
    await ui.checkForUpdate(false);
    expect(found()?.version).toBe('0.2.0-beta.1');
  });

  it('makes no automatic check when turned off, but "Check now" still works', async () => {
    await configure({ updateChannel: 'beta', updateChecks: 'off' });
    await ui.checkForUpdate(false);
    expect(found()).toBeNull();
    await ui.checkForUpdate(true);
    expect(found()?.version).toBe('0.2.0-beta.1');
  });
});
