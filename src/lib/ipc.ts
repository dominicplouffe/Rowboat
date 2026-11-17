// The only place the UI talks to the Rust core.
//
// In a plain browser (`npm run dev` without Tauri, or unit tests) there is no
// core, so development builds fall back to an in-memory stand-in. That branch
// is compiled out of production bundles.

import type {
  AppInfo,
  ConnectionConfig,
  ConnectionView,
  DbError,
  DbKind,
  FilePurpose,
  RunResult,
  SchemaNode,
  SecretInput,
  Settings,
  StatementSpan,
} from './types';

type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

let invokePromise: Promise<Invoke> | undefined;

function backend(): Promise<Invoke> {
  invokePromise ??= (async () => {
    if (inTauri) return (await import('@tauri-apps/api/core')).invoke as Invoke;
    if (import.meta.env.DEV || import.meta.env.MODE === 'test') {
      return (await import('./devBackend')).invoke as Invoke;
    }
    throw new Error('Rowboat UI must run inside the Rowboat app.');
  })();
  return invokePromise;
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return (await backend())<T>(command, args);
}

export const ipc = {
  appInfo: () => call<AppInfo>('app_info'),
  appReady: () => call<void>('app_ready'),
  settingsGet: () => call<Settings>('settings_get'),
  settingsSet: (settings: Settings) => call<Settings>('settings_set', { settings }),
  settingsReset: () => call<Settings>('settings_reset'),
  /** Resolves to the chosen path, or null when cancelled. */
  settingsExport: () => call<string | null>('settings_export'),
  /** Resolves to the imported settings, or null when cancelled. */
  settingsImport: () => call<Settings | null>('settings_import'),
  openDataFolder: () => call<void>('open_data_folder'),

  // Connections. These reject with a DbError (see `asDbError`).
  connectionsList: () => call<ConnectionView[]>('connections_list'),
  connectionSave: (config: ConnectionConfig, secrets: SecretInput) =>
    call<ConnectionView>('connection_save', { config, secrets }),
  connectionDelete: (id: string) => call<void>('connection_delete', { id }),
  /** Resolves to the server version. */
  connectionTest: (config: ConnectionConfig, secrets: SecretInput, trustedFingerprint?: string) =>
    call<string>('connection_test', { config, secrets, trustedFingerprint }),
  connectionOpen: (id: string, trustedFingerprint?: string) =>
    call<ConnectionView>('connection_open', { id, trustedFingerprint }),
  connectionClose: (id: string) => call<void>('connection_close', { id }),

  queryRun: (args: {
    connectionId: string;
    queryId: string;
    sql: string;
    rowLimit: number;
    confirmed: boolean;
  }) => call<RunResult>('query_run', args),
  queryCancel: (queryId: string) => call<void>('query_cancel', { queryId }),
  connectionSetAutoCommit: (connectionId: string, autoCommit: boolean) =>
    call<void>('connection_set_auto_commit', { connectionId, autoCommit }),
  transactionEnd: (connectionId: string, commit: boolean) =>
    call<void>('transaction_end', { connectionId, commit }),

  schemaChildren: (connectionId: string, path: string[]) =>
    call<SchemaNode[]>('schema_children', { connectionId, path }),
  schemaDdl: (connectionId: string, path: string[]) =>
    call<string>('schema_ddl', { connectionId, path }),
  sqlSplit: (kind: DbKind, text: string) => call<StatementSpan[]>('sql_split', { kind, text }),

  /** Opens a native file dialog; resolves to the path, or null when cancelled. */
  pickFile: (purpose: FilePurpose) => call<string | null>('pick_file', { purpose }),
};

/** Normalizes whatever a failed command threw into a DbError. */
export function asDbError(error: unknown): DbError {
  if (error && typeof error === 'object' && 'kind' in error && 'message' in error) {
    return error as DbError;
  }
  const message = error instanceof Error ? error.message : String(error);
  return { kind: 'other', message, detail: null, hint: null, code: null, position: null };
}
