// The in-memory schema index behind autocomplete, one per open connection.
// Completion never waits on the database: it reads whatever is cached and a
// refresh happens in the background.

import { buildIndex, type SchemaIndex } from './complete';
import { ipc } from './ipc';
import type { DbKind } from './types';

const USAGE_KEY = 'rowboat.completionUsage';

class SchemaCache {
  #indexes = new Map<string, SchemaIndex>();
  #loading = new Map<string, Promise<void>>();
  /** Bumped when an index changes, for the UI. */
  version = $state(0);
  /** How often each completion has been accepted. */
  usage: Record<string, number> = {};

  constructor() {
    try {
      this.usage = JSON.parse(localStorage.getItem(USAGE_KEY) ?? '{}');
    } catch {
      this.usage = {};
    }
  }

  get(connectionId: string | null | undefined): SchemaIndex | undefined {
    return connectionId ? this.#indexes.get(connectionId) : undefined;
  }

  /** Loads the schema if it is not cached yet. */
  ensure(connectionId: string, kind: DbKind): Promise<void> {
    if (this.#indexes.has(connectionId)) return Promise.resolve();
    return this.refresh(connectionId, kind);
  }

  /** Reloads the schema: on demand, and after statements that change it. */
  refresh(connectionId: string, kind: DbKind): Promise<void> {
    const running = this.#loading.get(connectionId);
    if (running) return running;
    const load = ipc
      .schemaSnapshot(connectionId)
      .then((snapshot) => {
        this.#indexes.set(connectionId, buildIndex(kind, snapshot));
        this.version++;
      })
      // Completion simply has less to offer if the schema cannot be read.
      .catch(() => {})
      .finally(() => this.#loading.delete(connectionId));
    this.#loading.set(connectionId, load);
    return load;
  }

  forget(connectionId: string): void {
    this.#indexes.delete(connectionId);
  }

  /** Records that a completion was accepted, for ranking. */
  used(usageKey: string): void {
    this.usage[usageKey] = (this.usage[usageKey] ?? 0) + 1;
    try {
      localStorage.setItem(USAGE_KEY, JSON.stringify(this.usage));
    } catch {
      // Ranking by use is a nicety; it works without storage for this session.
    }
  }
}

export const schemaCache = new SchemaCache();

/** Statements after which the cached schema is stale. */
export function changesSchema(sql: string): boolean {
  return /^\s*(create|alter|drop|rename|truncate|attach|detach|comment)\b/i.test(
    sql.replace(/^(\s*(--[^\n]*\n|\/\*[\s\S]*?\*\/))*/g, ''),
  );
}
