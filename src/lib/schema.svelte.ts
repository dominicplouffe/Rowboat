// The schema navigator's lazily loaded tree, one per open connection.

import { asDbError, ipc } from './ipc';
import type { DbKind, SchemaNode } from './types';

interface Branch {
  status: 'loading' | 'ready' | 'failed';
  nodes: SchemaNode[];
  error?: string;
}

const SEPARATOR = '\u0000';

class Schema {
  branches = $state<Record<string, Branch>>({});
  expanded = $state<Record<string, boolean>>({});

  key(connectionId: string, path: string[]): string {
    return [connectionId, ...path].join(SEPARATOR);
  }

  branch(connectionId: string, path: string[]): Branch | undefined {
    return this.branches[this.key(connectionId, path)];
  }

  isExpanded(connectionId: string, path: string[]): boolean {
    return this.expanded[this.key(connectionId, path)] ?? false;
  }

  async load(connectionId: string, path: string[]): Promise<void> {
    const key = this.key(connectionId, path);
    this.branches[key] = { status: 'loading', nodes: this.branches[key]?.nodes ?? [] };
    try {
      this.branches[key] = { status: 'ready', nodes: await ipc.schemaChildren(connectionId, path) };
    } catch (e) {
      this.branches[key] = { status: 'failed', nodes: [], error: asDbError(e).message };
    }
  }

  async toggle(connectionId: string, path: string[]): Promise<void> {
    const key = this.key(connectionId, path);
    this.expanded[key] = !this.expanded[key];
    if (this.expanded[key] && !this.branches[key]) await this.load(connectionId, path);
  }

  /** Forgets everything under a path (or a whole connection) and reloads what is open. */
  async refresh(connectionId: string, path: string[] = []): Promise<void> {
    const prefix = this.key(connectionId, path);
    const open: string[][] = [];
    for (const key of Object.keys(this.branches)) {
      if (key === prefix || key.startsWith(prefix + SEPARATOR)) {
        if (this.expanded[key] || key === prefix) open.push(key.split(SEPARATOR).slice(1));
        delete this.branches[key];
      }
    }
    await Promise.all(open.map((p) => this.load(connectionId, p)));
  }

  /** Drops a closed or deleted connection's tree. */
  forget(connectionId: string): void {
    for (const store of [this.branches, this.expanded]) {
      for (const key of Object.keys(store)) {
        if (key === connectionId || key.startsWith(connectionId + SEPARATOR)) delete store[key];
      }
    }
  }
}

export const schema = new Schema();

const BARE = /^[a-z_][a-z0-9_]*$/;

/** Quotes an identifier only when the database would need it. */
export function quoteIdent(kind: DbKind, name: string): string {
  if (BARE.test(name)) return name;
  return kind === 'clickhouse'
    ? `\`${name.replace(/\\/g, '\\\\').replace(/`/g, '\\`')}\``
    : `"${name.replace(/"/g, '""')}"`;
}

/** The SQL name of the table or view at a navigator path. */
export function qualifiedName(kind: DbKind, path: string[]): string {
  // PostgreSQL / ClickHouse: [schema, group, object]; SQLite: [group, object].
  const parts = kind === 'sqlite' ? [path[1]] : [path[0], path[2]];
  return parts.map((p) => quoteIdent(kind, p)).join('.');
}

export function selectTemplate(kind: DbKind, path: string[], columns: string[]): string {
  const list =
    columns.length > 0 ? columns.map((c) => quoteIdent(kind, c)).join(',\n       ') : '*';
  return `SELECT ${list}\nFROM ${qualifiedName(kind, path)};\n`;
}

export function insertTemplate(kind: DbKind, path: string[], columns: string[]): string {
  const names = columns.map((c) => quoteIdent(kind, c)).join(', ');
  const values = columns.map((c) => `/* ${c} */ NULL`).join(',\n        ');
  return `INSERT INTO ${qualifiedName(kind, path)} (${names})\nVALUES (${values});\n`;
}
