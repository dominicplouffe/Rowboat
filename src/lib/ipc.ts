// The only place the UI talks to the Rust core.
//
// In a plain browser (`npm run dev` without Tauri, or unit tests) there is no
// core, so development builds fall back to an in-memory stand-in. That branch
// is compiled out of production bundles.

import type {
  AiError,
  AiEvent,
  AiOutcome,
  AiProvider,
  AiRequest,
  AppInfo,
  Column,
  ConnectionConfig,
  ConnectionView,
  DbError,
  DbKind,
  EditTarget,
  ExportFormat,
  FilePurpose,
  HistoryEntry,
  IndexInfo,
  RowEdit,
  RunOptions,
  RunResult,
  SchemaNode,
  SchemaSnapshot,
  SecretInput,
  Settings,
  StatementSpan,
  UpdateInfo,
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

  queryRun: (args: { connectionId: string; queryId: string; sql: string; options: RunOptions }) =>
    call<RunResult>('query_run', args),
  /** Counts the rows a query returns without fetching them. */
  queryCount: (connectionId: string, queryId: string, sql: string) =>
    call<number>('query_count', { connectionId, queryId, sql }),
  schemaSnapshot: (connectionId: string) =>
    call<SchemaSnapshot>('schema_snapshot', { connectionId }),

  /** Re-runs `sql` with no row limit, streaming to a file the user picks. Null when cancelled. */
  exportQuery: (args: {
    connectionId: string;
    queryId: string;
    sql: string;
    format: ExportFormat;
    table: string;
  }) => call<{ path: string; rows: number } | null>('export_query', args),
  /** Writes rows the UI already holds to a file the user picks. */
  exportPage: (args: {
    format: ExportFormat;
    kind: DbKind;
    table: string;
    columns: Column[];
    rows: (string | null)[][];
  }) => call<{ path: string; rows: number } | null>('export_page', args),
  /** Formats rows as text, for the clipboard. */
  formatRows: (args: {
    format: ExportFormat;
    kind: DbKind;
    table: string;
    columns: Column[];
    rows: (string | null)[][];
  }) => call<string>('format_rows', args),

  editsPreview: (connectionId: string, target: EditTarget, edits: RowEdit[]) =>
    call<string[]>('edits_preview', { connectionId, target, edits }),
  editsApply: (connectionId: string, queryId: string, target: EditTarget, edits: RowEdit[]) =>
    call<number>('edits_apply', { connectionId, queryId, target, edits }),

  historySearch: (connectionId: string | null, search: string, limit = 200) =>
    call<HistoryEntry[]>('history_search', { connectionId, search, limit }),
  historyClear: () => call<void>('history_clear'),

  sessionSave: (value: string) => call<void>('session_save', { value }),
  sessionLoad: () => call<string | null>('session_load'),

  sqlFileOpen: () => call<{ path: string; text: string } | null>('sql_file_open'),
  /** Saves to `path`, or asks where when it is null. Resolves to the path, or null when cancelled. */
  sqlFileSave: (path: string | null, text: string, name: string) =>
    call<string | null>('sql_file_save', { path, text, name }),
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

  // AI. These reject with an AiError (see `asAiError`). A key goes in once
  // and is never read back.
  aiKeySet: (providerId: string, key: string) => call<void>('ai_key_set', { providerId, key }),
  aiKeyStatus: () => call<Record<string, boolean>>('ai_key_status'),
  aiModels: (provider: AiProvider) => call<string[]>('ai_models', { provider }),
  /** One small real request; resolves to the model's reply. */
  aiTest: (provider: AiProvider) => call<string>('ai_test', { provider }),
  /** Ask, Fix or Explain. `onEvent` receives the payload sent and the answer as it streams. */
  aiRun: async (request: AiRequest, onEvent: (event: AiEvent) => void) => {
    if (!inTauri) return call<AiOutcome>('ai_run', { request, onEvent });
    const { Channel } = await import('@tauri-apps/api/core');
    const channel = new Channel<AiEvent>();
    channel.onmessage = onEvent;
    return call<AiOutcome>('ai_run', { request, onEvent: channel });
  },
  aiCancel: (id: string) => call<void>('ai_cancel', { id }),
  /** Builds (or with `refresh`, rebuilds) the schema index. Null when there is none and the connection is closed. */
  aiIndex: (connectionId: string, refresh: boolean) =>
    call<IndexInfo | null>('ai_index', { connectionId, refresh }),

  /**
   * Asks GitHub whether a newer release exists; null when there is none or
   * when update checks are off (`manual` overrides that). Rejects with a string.
   */
  updateCheck: (manual: boolean) => call<UpdateInfo | null>('update_check', { manual }),
  /** Opens a Rowboat release page in the browser. */
  updateOpen: (url: string) => call<void>('update_open', { url }),

  /** Creates the sample SQLite database and a connection to it. */
  sampleDatabase: () => call<ConnectionView>('sample_database'),
};

/** Normalizes whatever a failed AI command threw into an AiError. */
export function asAiError(error: unknown): AiError {
  if (error && typeof error === 'object' && 'kind' in error && 'message' in error) {
    return error as AiError;
  }
  return { kind: 'other', message: error instanceof Error ? error.message : String(error) };
}

/** Normalizes whatever a failed command threw into a DbError. */
export function asDbError(error: unknown): DbError {
  if (error && typeof error === 'object' && 'kind' in error && 'message' in error) {
    return error as DbError;
  }
  const message = error instanceof Error ? error.message : String(error);
  return { kind: 'other', message, detail: null, hint: null, code: null, position: null };
}
