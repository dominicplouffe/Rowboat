// Open editor tabs. Each tab has its own text, row limit, connection and results.

import type { AiError, AiEvent, AiOutcome, DbError, RunResult } from './types';

/** One executed statement and how it went. */
export interface StatementRun {
  sql: string;
  /** Offset of the statement in the editor text (UTF-16), for error markers. */
  offset: number;
  status: 'pending' | 'running' | 'done' | 'failed' | 'skipped';
  result?: RunResult;
  error?: DbError;
  /** Total rows, once "Count rows" has been asked for. */
  count?: number;
  /** What a follow-up (next page, fetch all, count, export, apply) is doing, if anything. */
  busy?: 'page' | 'all' | 'count' | 'export' | 'apply';
  /** Shown under the grid: the outcome of the last follow-up. */
  note?: string;
  /** Set when this ran as EXPLAIN, to show the plan as a tree. */
  plan?: boolean;
  /** Staged cell edits: row index → column index → new value (null = NULL). */
  edits?: Record<number, Record<number, string | null>>;
  /** The AI's suggested fix for a failed statement. */
  fix?: FixState;
}

export interface FixState {
  /** Request id, for cancel. */
  id: string;
  status: 'loading' | 'ready' | 'failed' | 'accepted' | 'dismissed';
  /** The answer as it streams in. */
  text: string;
  outcome?: AiOutcome;
  error?: AiError;
  context?: Extract<AiEvent, { type: 'context' }>;
}

export interface Tab {
  id: number;
  title: string;
  text: string;
  /** Rows per fetch for this tab; 0 means no limit. */
  rowLimit: number;
  connectionId: string | null;
  /** Statements from the last run, in order. */
  runs: StatementRun[];
  /** Which of `runs` the results pane shows. */
  shown: number;
  running: boolean;
  /** Id of the statement in flight, for cancel. */
  queryId: string | null;
  /** Read-only tabs show generated text (DDL) and cannot be run by accident. */
  generated: boolean;
  /** The `.sql` file this tab was opened from or saved to. */
  path: string | null;
  /** Text as last saved to `path`, to know whether there are unsaved changes. */
  savedText: string | null;
}

class Tabs {
  list = $state<Tab[]>([]);
  activeId = $state(0);
  #nextId = 1;

  get active(): Tab | undefined {
    return this.list.find((tab) => tab.id === this.activeId);
  }

  open(
    rowLimit: number,
    text = '',
    options: Partial<Pick<Tab, 'connectionId' | 'title' | 'generated' | 'path'>> = {},
  ): Tab {
    const id = this.#nextId++;
    this.list.push({
      id,
      title: options.title ?? `Query ${id}`,
      text,
      rowLimit,
      // A new tab starts on the current tab's connection unless told otherwise
      // (an explicit null means "no connection").
      connectionId:
        'connectionId' in options
          ? (options.connectionId ?? null)
          : (this.active?.connectionId ?? null),
      runs: [],
      shown: 0,
      running: false,
      queryId: null,
      generated: options.generated ?? false,
      path: options.path ?? null,
      savedText: options.path ? text : null,
    });
    this.activeId = id;
    // Return the reactive proxy stored in the list, not the plain object.
    return this.list[this.list.length - 1];
  }

  close(id: number): void {
    const index = this.list.findIndex((tab) => tab.id === id);
    if (index === -1) return;
    this.list.splice(index, 1);
    if (this.activeId === id) {
      this.activeId = this.list[Math.min(index, this.list.length - 1)]?.id ?? 0;
    }
  }

  /** Moves the selection by `delta` tabs, wrapping around. */
  cycle(delta: number): void {
    if (this.list.length === 0) return;
    const index = this.list.findIndex((tab) => tab.id === this.activeId);
    const next = (index + delta + this.list.length) % this.list.length;
    this.activeId = this.list[next].id;
  }

  /** Detaches tabs from a connection that was deleted. */
  forgetConnection(connectionId: string): void {
    for (const tab of this.list) {
      if (tab.connectionId === connectionId) tab.connectionId = null;
    }
  }
}

export const tabs = new Tabs();
