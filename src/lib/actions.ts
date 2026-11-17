// Every keyboard-reachable action in the app. Shown in Settings → Keyboard.

import { connections } from './connections.svelte';
import { dialogs } from './dialogs.svelte';
import { editorBridge } from './editorBridge';
import { formatSql } from './format';
import { ipc } from './ipc';
import { cancel, explain, run, type RunScope } from './runner';
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

/** A file name without its folders. */
const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

async function formatActive(): Promise<void> {
  const tab = tabs.active;
  if (!tab || tab.generated || !settings.value) return;
  const { from, to } = editorBridge.selection();
  const whole = from === to;
  const source = whole ? editorBridge.text() : editorBridge.text().slice(from, to);
  if (!source.trim()) return;
  try {
    const kind = connections.byId(tab.connectionId)?.kind ?? null;
    const formatted = await formatSql(source, kind, settings.value.editor);
    if (formatted !== source)
      editorBridge.replace(formatted, whole ? 0 : from, whole ? undefined : to);
  } catch {
    // The formatter could not parse this SQL; leave the text exactly as it is.
  }
}

async function openFile(): Promise<void> {
  const file = await ipc.sqlFileOpen().catch(() => null);
  if (!file) return;
  tabs.open(settings.value?.execution.defaultRowLimit ?? 1000, file.text, {
    title: baseName(file.path),
    path: file.path,
  });
}

async function saveFile(): Promise<void> {
  const tab = tabs.active;
  if (!tab || tab.generated) return;
  const path = await ipc
    .sqlFileSave(tab.path, tab.text, tab.title.replace(/\.sql$/i, ''))
    .catch(() => null);
  if (!path) return;
  tab.path = path;
  tab.title = baseName(path);
  tab.savedText = tab.text;
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
    id: 'query.explain',
    group: 'Query',
    label: 'Explain statement (show the plan)',
    chord: 'Mod+E',
    run: () => {
      const tab = tabs.active;
      if (!tab || !connections.byId(tab.connectionId)) return;
      editorBridge.markError(null);
      void explain(tab, editorBridge.selection());
    },
  },
  {
    id: 'editor.format',
    group: 'Editor',
    label: 'Format SQL',
    chord: 'Mod+Shift+F',
    run: () => void formatActive(),
  },
  {
    id: 'file.open',
    group: 'File',
    label: 'Open SQL file…',
    chord: 'Mod+O',
    run: () => void openFile(),
  },
  {
    id: 'file.save',
    group: 'File',
    label: 'Save as SQL file',
    chord: 'Mod+S',
    run: () => void saveFile(),
  },
  {
    id: 'history.open',
    group: 'Query',
    label: 'Query history',
    chord: 'Mod+Y',
    run: () => (ui.historyOpen = true),
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
