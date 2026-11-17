// Every keyboard-reachable action in the app. Shown in Settings → Keyboard.

import { connections } from './connections.svelte';
import { dialogs } from './dialogs.svelte';
import { editorBridge } from './editorBridge';
import { cancel, run, type RunScope } from './runner';
import { settings } from './settings.svelte';
import type { Action } from './shortcuts';
import { tabs } from './tabs.svelte';
import { ui } from './ui.svelte';

const newTab = () => tabs.open(settings.value?.execution.defaultRowLimit ?? 1000);

function runActive(scope: RunScope): void {
  const tab = tabs.active;
  if (!tab || tab.running || tab.generated) return;
  if (!connections.byId(tab.connectionId)) {
    // Nothing to run against: offer to create a connection instead.
    if (connections.list.length === 0) dialogs.connection = { editing: null };
    return;
  }
  editorBridge.markError(null);
  void run(tab, editorBridge.selection(), scope);
}

export const ACTIONS: Action[] = [
  {
    id: 'query.run',
    group: 'Query',
    label: 'Run statement or selection',
    chord: 'Mod+Enter',
    run: () => runActive('statement'),
  },
  {
    id: 'query.runScript',
    group: 'Query',
    label: 'Run script',
    chord: 'Mod+Shift+Enter',
    run: () => runActive('script'),
  },
  {
    id: 'query.cancel',
    group: 'Query',
    label: 'Cancel running query',
    chord: 'Mod+.',
    run: () => {
      const tab = tabs.active;
      if (tab?.running) void cancel(tab);
    },
  },
  {
    id: 'connection.new',
    group: 'Connections',
    label: 'New connection',
    chord: 'Mod+Shift+N',
    run: () => void (dialogs.connection = { editing: null }),
  },
  {
    id: 'tab.new',
    group: 'Tabs',
    label: 'New query tab',
    chord: 'Mod+T',
    run: () => void newTab(),
  },
  {
    id: 'tab.close',
    group: 'Tabs',
    label: 'Close tab',
    chord: 'Mod+W',
    run: () => tabs.close(tabs.activeId),
  },
  {
    id: 'tab.next',
    group: 'Tabs',
    label: 'Next tab',
    chord: 'Mod+Alt+ArrowRight',
    run: () => tabs.cycle(1),
  },
  {
    id: 'tab.previous',
    group: 'Tabs',
    label: 'Previous tab',
    chord: 'Mod+Alt+ArrowLeft',
    run: () => tabs.cycle(-1),
  },
  {
    id: 'sidebar.toggle',
    group: 'Window',
    label: 'Show or hide the sidebar',
    chord: 'Mod+B',
    run: () => (ui.sidebarVisible = !ui.sidebarVisible),
  },
  {
    id: 'settings.open',
    group: 'Window',
    label: 'Open Settings',
    chord: 'Mod+,',
    run: () => ui.openSettings(),
  },
];
