// Open editor tabs. Each tab has its own text, row limit, connection and results.

import type { DbError, RunResult } from './types';

/** One executed statement and how it went. */
export interface StatementRun {
  sql: string;
  /** Offset of the statement in the editor text (UTF-16), for error markers. */
  offset: number;
  status: 'pending' | 'running' | 'done' | 'failed' | 'skipped';
  result?: RunResult;
  error?: DbError;
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
    options: Partial<Pick<Tab, 'connectionId' | 'title' | 'generated'>> = {},
  ): Tab {
    const id = this.#nextId++;
    this.list.push({
      id,
      title: options.title ?? `Query ${id}`,
      text,
      rowLimit,
      connectionId: options.connectionId ?? this.active?.connectionId ?? null,
      runs: [],
      shown: 0,
      running: false,
      queryId: null,
      generated: options.generated ?? false,
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
