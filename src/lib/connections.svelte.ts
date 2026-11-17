// Saved connections and their open/closed state, mirrored from the core.

import { asDbError, ipc } from './ipc';
import { schemaCache } from './schemaCache.svelte';
import type { ConnectionConfig, ConnectionView, DbError, DbKind, SecretInput } from './types';

export const DEFAULT_PORTS: Record<DbKind, number> = {
  postgres: 5432,
  clickhouse: 8123,
  sqlite: 0,
};

export const KIND_LABELS: Record<DbKind, string> = {
  postgres: 'PostgreSQL',
  clickhouse: 'ClickHouse',
  sqlite: 'SQLite',
};

export function blankConnection(kind: DbKind = 'postgres'): ConnectionConfig {
  return {
    id: '',
    name: '',
    kind,
    folder: '',
    color: '',
    environment: 'none',
    readOnly: false,
    host: 'localhost',
    port: DEFAULT_PORTS[kind],
    database: '',
    user: '',
    usePgpass: true,
    tls: {
      mode: kind === 'clickhouse' ? 'disable' : 'prefer',
      caCertPath: '',
      clientCertPath: '',
      clientKeyPath: '',
    },
    ssh: { enabled: false, host: '', port: 22, user: '', auth: 'agent', keyPath: '' },
    path: '',
    create: false,
  };
}

/** Thrown to the caller when the SSH host key needs the user's approval. */
export interface HostKeyPrompt {
  message: string;
  fingerprint: string;
  keyType: string;
}

class Connections {
  list = $state<ConnectionView[]>([]);
  /** Connections currently being opened. */
  opening = $state<Record<string, boolean>>({});
  /** The last connect error per connection, shown in the sidebar. */
  errors = $state<Record<string, DbError>>({});
  /** Set by the UI to ask the user about an unknown SSH host key. */
  askHostKey: (prompt: HostKeyPrompt) => Promise<boolean> = async () => false;

  byId(id: string | null | undefined): ConnectionView | undefined {
    return this.list.find((c) => c.id === id);
  }

  /** Folder names in use, for the dialog's suggestions. */
  get folders(): string[] {
    return [...new Set(this.list.map((c) => c.folder).filter(Boolean))].sort();
  }

  async load(): Promise<void> {
    this.list = await ipc.connectionsList();
  }

  #replace(view: ConnectionView): void {
    const index = this.list.findIndex((c) => c.id === view.id);
    if (index === -1) this.list.push(view);
    else this.list[index] = view;
  }

  async save(config: ConnectionConfig, secrets: SecretInput): Promise<ConnectionView> {
    const view = await ipc.connectionSave(config, secrets);
    this.#replace(view);
    delete this.errors[view.id];
    return view;
  }

  async remove(id: string): Promise<void> {
    await ipc.connectionDelete(id);
    this.list = this.list.filter((c) => c.id !== id);
  }

  /**
   * Runs `attempt`; if it fails on an unknown SSH host key, asks the user and
   * retries once with the fingerprint they approved.
   */
  async withHostKeyPrompt<T>(attempt: (trustedFingerprint?: string) => Promise<T>): Promise<T> {
    try {
      return await attempt();
    } catch (e) {
      const error = asDbError(e);
      if (error.kind !== 'unknown-host-key' || !error.detail) throw error;
      const trusted = await this.askHostKey({
        message: error.message,
        fingerprint: error.detail,
        keyType: error.hint ?? '',
      });
      if (!trusted) throw error;
      return await attempt(error.detail);
    }
  }

  /** Opens a connection. Resolves to false (with `errors[id]` set) on failure. */
  async open(id: string): Promise<boolean> {
    if (this.byId(id)?.connected) return true;
    if (this.opening[id]) return false;
    this.opening[id] = true;
    delete this.errors[id];
    try {
      this.#replace(await this.withHostKeyPrompt((fp) => ipc.connectionOpen(id, fp)));
      return true;
    } catch (e) {
      this.errors[id] = asDbError(e);
      return false;
    } finally {
      delete this.opening[id];
    }
  }

  async close(id: string): Promise<void> {
    await ipc.connectionClose(id);
    schemaCache.forget(id);
    const view = this.byId(id);
    if (view) {
      view.connected = false;
      view.inTransaction = false;
      view.serverVersion = null;
    }
  }

  setInTransaction(id: string, inTransaction: boolean): void {
    const view = this.byId(id);
    if (view) view.inTransaction = inTransaction;
  }

  async setAutoCommit(id: string, autoCommit: boolean): Promise<void> {
    await ipc.connectionSetAutoCommit(id, autoCommit);
    const view = this.byId(id);
    if (view) view.autoCommit = autoCommit;
  }

  /**
   * A statement failed because the connection (or its SSH tunnel) is gone.
   * The dead session is dropped in the core too, so that the next Connect or
   * Run opens a fresh one instead of reusing it.
   */
  async dropDead(id: string): Promise<void> {
    await ipc.connectionClose(id).catch(() => {});
    schemaCache.forget(id);
    const view = this.byId(id);
    if (view) {
      view.connected = false;
      view.inTransaction = false;
      view.serverVersion = null;
    }
  }
}

export const connections = new Connections();
