// Session restore: open tabs and their unsaved text are written to the app's
// own database shortly after every change, so they survive restarts and
// crashes. Results are not saved, only what the user typed.

import { ipc } from './ipc';
import { tabs, type Tab } from './tabs.svelte';

interface SavedTab {
  title: string;
  text: string;
  connectionId: string | null;
  rowLimit: number;
  path: string | null;
}

interface SavedSession {
  tabs: SavedTab[];
  active: number;
}

export function serialize(list: Tab[], activeId: number): SavedSession {
  // Generated tabs (DDL) are views of the database, not the user's work.
  const kept = list.filter((tab) => !tab.generated);
  return {
    tabs: kept.map(({ title, text, connectionId, rowLimit, path }) => ({
      title,
      text,
      connectionId,
      rowLimit,
      path,
    })),
    active: Math.max(
      0,
      kept.findIndex((tab) => tab.id === activeId),
    ),
  };
}

/** Parses a saved session, ignoring anything malformed. */
export function parse(json: string | null): SavedSession | null {
  if (!json) return null;
  try {
    const data = JSON.parse(json) as Partial<SavedSession>;
    if (!Array.isArray(data.tabs)) return null;
    const restored = data.tabs
      .filter((tab) => tab && typeof tab.text === 'string')
      .map((tab) => ({
        title: typeof tab.title === 'string' ? tab.title : 'Query',
        text: tab.text,
        connectionId: typeof tab.connectionId === 'string' ? tab.connectionId : null,
        rowLimit: Number.isInteger(tab.rowLimit) && tab.rowLimit >= 0 ? tab.rowLimit : 1000,
        path: typeof tab.path === 'string' ? tab.path : null,
      }));
    if (restored.length === 0) return null;
    const active = Number.isInteger(data.active)
      ? Math.min(Math.max(data.active!, 0), restored.length - 1)
      : 0;
    return { tabs: restored, active };
  } catch {
    return null;
  }
}

let timer: ReturnType<typeof setTimeout> | undefined;
let lastSaved = '';

/** Saves the session a moment from now; rapid changes are saved once. */
export function saveSoon(): void {
  clearTimeout(timer);
  timer = setTimeout(() => {
    const json = JSON.stringify(serialize(tabs.list, tabs.activeId));
    if (json === lastSaved) return;
    lastSaved = json;
    void ipc.sessionSave(json).catch(() => {});
  }, 400);
}

/** Loads the saved session text (run alongside the other startup requests). */
export const loadSaved = (): Promise<string | null> => ipc.sessionLoad().catch(() => null);

/** Reopens the saved tabs. Returns false when there was nothing to restore. */
export function restore(knownConnections: Set<string>, saved: string | null): boolean {
  const session = parse(saved);
  if (!session) return false;
  const opened = session.tabs.map((saved) => {
    const tab = tabs.open(saved.rowLimit, saved.text, {
      title: saved.title,
      // A connection deleted since then is simply not selected.
      connectionId:
        saved.connectionId && knownConnections.has(saved.connectionId) ? saved.connectionId : null,
      path: saved.path,
    });
    // Text restored from a crash may differ from the file on disk.
    tab.savedText = null;
    return tab;
  });
  tabs.activeId = opened[session.active].id;
  return true;
}
