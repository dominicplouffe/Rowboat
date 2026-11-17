// In-memory stand-in for the Rust core, for browser-only development and
// tests. Never part of a production bundle (see ipc.ts).

import type {
  AiEvent,
  AiOutcome,
  AiProvider,
  AiRequest,
  AppInfo,
  ConnectionConfig,
  ConnectionView,
  DbError,
  HistoryEntry,
  RunOptions,
  SchemaSnapshot,
  RunResult,
  SchemaNode,
  SecretInput,
  Settings,
  StatementSpan,
} from './types';

export function defaultSettings(): Settings {
  return {
    version: 1,
    general: {
      clientId: 'rowboat-dev',
      restoreTabs: true,
      updateChecks: 'auto',
      updateChannel: 'stable',
      onboardingCompleted: false,
    },
    appearance: {
      theme: 'system',
      density: 'comfortable',
      editorFontFamily: '',
      editorFontSize: 13,
      gridFontFamily: '',
      gridFontSize: 12,
    },
    editor: {
      tabSize: 2,
      indentWithTabs: false,
      wordWrap: false,
      lineNumbers: true,
      autoCloseBrackets: true,
      autocomplete: 'typing',
      formatterKeywordCase: 'upper',
    },
    execution: {
      defaultRowLimit: 1000,
      queryTimeoutSecs: 0,
      autoCommit: true,
      stopScriptOnError: true,
      confirmDestructive: 'prod-only',
      resultMemoryCapMb: 256,
    },
    results: {
      nullText: 'NULL',
      dateTimeFormat: 'YYYY-MM-DD HH:mm:ss',
      timeZone: 'local',
      numberFormat: 'plain',
      binaryDisplay: 'hex',
      maxCellChars: 500,
      jsonAutoDetect: true,
    },
    connections: {
      connectTimeoutSecs: 10,
      keepaliveSecs: 30,
      autoReconnect: true,
      sshKnownHosts: 'ask',
    },
    ai: {
      providers: [],
      defaultProvider: '',
      autoSuggestFixes: true,
      sendSampleValues: false,
      schemaTokenBudget: 4000,
      requestTimeoutSecs: 60,
    },
    keyboard: { overrides: {} },
    privacy: { keepHistory: true, historyRetentionDays: 90 },
  };
}

let current = defaultSettings();

const info: AppInfo = {
  name: 'Rowboat',
  version: '0.1.0-dev',
  os: 'browser',
  arch: 'n/a',
  defaultClientId: 'rowboat-dev',
  settingsPath: '(in memory)',
  secretsPersistent: false,
};

export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  switch (command) {
    case 'app_info':
      return info as T;
    case 'app_ready':
    case 'open_data_folder':
      return undefined as T;
    case 'settings_get':
      return structuredClone(current) as T;
    case 'settings_set':
      current = structuredClone(args!.settings as Settings);
      return structuredClone(current) as T;
    case 'settings_reset':
      current = { ...defaultSettings(), general: current.general };
      return structuredClone(current) as T;
    case 'settings_export':
    case 'settings_import':
      return null as T;
    default:
      return fakeDatabase<T>(command, args ?? {});
  }
}

// ---- A pretend database, enough to exercise the UI without a server ----

const connections = new Map<string, ConnectionConfig>();
const withPassword = new Set<string>();
const open = new Set<string>();
const cancelled = new Set<string>();
const manualCommit = new Set<string>();
const history: HistoryEntry[] = [];
let savedSession: string | null = null;

const SNAPSHOT: SchemaSnapshot = {
  searchPath: ['public'],
  tables: [
    {
      schema: 'public',
      name: 'customers',
      kind: 'table',
      columns: [
        { name: 'id', typeName: 'integer', primaryKey: true },
        { name: 'name', typeName: 'text', primaryKey: false },
      ],
      foreignKeys: [],
    },
    {
      schema: 'public',
      name: 'orders',
      kind: 'table',
      columns: [
        { name: 'id', typeName: 'integer', primaryKey: true },
        { name: 'customer_id', typeName: 'integer', primaryKey: false },
        { name: 'total', typeName: 'numeric(10,2)', primaryKey: false },
        { name: 'created_at', typeName: 'timestamp with time zone', primaryKey: false },
      ],
      foreignKeys: [
        {
          columns: ['customer_id'],
          refSchema: 'public',
          refTable: 'customers',
          refColumns: ['id'],
        },
      ],
    },
  ],
  functions: [
    { name: 'count', signature: '(*)' },
    { name: 'lower', signature: '(text)' },
  ],
};
let nextId = 1;

function fail(kind: DbError['kind'], message: string, extra: Partial<DbError> = {}): DbError {
  return { kind, message, detail: null, hint: null, code: null, position: null, ...extra };
}

function view(config: ConnectionConfig): ConnectionView {
  return {
    ...config,
    hasPassword: withPassword.has(config.id),
    hasSshPassword: false,
    hasSshPassphrase: false,
    connected: open.has(config.id),
    serverVersion: open.has(config.id) ? 'Fake DB 1.0' : null,
    supportsTransactions: config.kind !== 'clickhouse',
    inTransaction: false,
    autoCommit: !manualCommit.has(config.id),
  };
}

/** Naive splitter for the fake only; the real one is in the Rust core. */
function split(text: string): StatementSpan[] {
  const spans: StatementSpan[] = [];
  let start = 0;
  for (const part of text.split(';')) {
    const lead = part.length - part.trimStart().length;
    const body = part.trim();
    if (body) spans.push({ start: start + lead, end: start + lead + body.length });
    start += part.length + 1;
  }
  return spans;
}

const TREE: Record<string, SchemaNode[]> = {
  '': [{ name: 'public', kind: 'schema', detail: null, hasChildren: true }],
  public: [{ name: 'Tables', kind: 'group', detail: null, hasChildren: true }],
  'public/Tables': [
    { name: 'customers', kind: 'table', detail: '~1.2k rows', hasChildren: true },
    { name: 'orders', kind: 'table', detail: '~48.0k rows', hasChildren: true },
  ],
  'public/Tables/orders': [
    { name: 'id', kind: 'column', detail: 'integer · primary key', hasChildren: false },
    { name: 'customer_id', kind: 'column', detail: 'integer · not null', hasChildren: false },
    { name: 'total', kind: 'column', detail: 'numeric(10,2)', hasChildren: false },
    { name: 'created_at', kind: 'column', detail: 'timestamp with time zone', hasChildren: false },
  ],
  'public/Tables/customers': [
    { name: 'id', kind: 'column', detail: 'integer · primary key', hasChildren: false },
    { name: 'name', kind: 'column', detail: 'text · not null', hasChildren: false },
  ],
};

async function fakeDatabase<T>(command: string, args: Record<string, unknown>): Promise<T> {
  switch (command) {
    case 'connections_list':
      return [...connections.values()].map(view) as T;
    case 'connection_save': {
      const config = structuredClone(args.config as ConnectionConfig);
      if (!config.name.trim()) throw fail('other', 'Give the connection a name.');
      if (!config.id) config.id = `fake-${nextId++}`;
      else open.delete(config.id);
      connections.set(config.id, config);
      const password = (args.secrets as SecretInput).password;
      if (password !== undefined) {
        if (password) withPassword.add(config.id);
        else withPassword.delete(config.id);
      }
      return view(config) as T;
    }
    case 'connection_delete':
      connections.delete(args.id as string);
      open.delete(args.id as string);
      return undefined as T;
    case 'connection_test': {
      const config = args.config as ConnectionConfig;
      if (config.host === 'unreachable') throw fail('connection', 'Connection refused.');
      return 'Fake DB 1.0' as T;
    }
    case 'connection_open': {
      const config = connections.get(args.id as string);
      if (!config) throw fail('other', 'That connection no longer exists.');
      if (config.host === 'unreachable') throw fail('connection', 'Connection refused.');
      open.add(config.id);
      return view(config) as T;
    }
    case 'connection_close':
      open.delete(args.connectionId as string);
      open.delete(args.id as string);
      return undefined as T;
    case 'sql_split':
      return split(args.text as string) as T;
    case 'query_cancel':
      cancelled.add(args.queryId as string);
      return undefined as T;
    case 'connection_set_auto_commit':
      if (args.autoCommit) manualCommit.delete(args.connectionId as string);
      else manualCommit.add(args.connectionId as string);
      return undefined as T;
    case 'transaction_end':
    case 'pick_file':
      return (command === 'pick_file' ? null : undefined) as T;
    case 'schema_snapshot':
      return SNAPSHOT as T;
    case 'query_count':
      return 5000 as T;
    case 'export_query':
    case 'export_page':
    case 'sql_file_open':
    case 'sql_file_save':
      return null as T;
    case 'format_rows':
      return (args.rows as (string | null)[][]).map((row) => row.join('\t')).join('\n') as T;
    case 'edits_preview':
      return (args.edits as { key: string[]; changes: [string, string | null][] }[]).map(
        (edit) =>
          `UPDATE orders SET ${edit.changes
            .map(([c, v]) => `${c} = ${v === null ? 'NULL' : `'${v}'`}`)
            .join(', ')} WHERE id = '${edit.key[0]}'`,
      ) as T;
    case 'edits_apply':
      return (args.edits as unknown[]).length as T;
    case 'history_search': {
      const words = (args.search as string).toLowerCase().split(/\s+/).filter(Boolean);
      return history
        .filter((h) => !args.connectionId || h.connectionId === args.connectionId)
        .filter((h) => words.every((w) => h.sql.toLowerCase().includes(w)))
        .slice(0, args.limit as number) as T;
    }
    case 'history_clear':
      history.length = 0;
      return undefined as T;
    case 'session_save':
      savedSession = args.value as string;
      return undefined as T;
    case 'session_load':
      return savedSession as T;
    case 'schema_children':
      return (TREE[(args.path as string[]).join('/')] ?? []) as T;
    case 'schema_ddl':
      return `CREATE TABLE ${(args.path as string[]).at(-1)} (\n    id integer PRIMARY KEY\n);` as T;
    case 'query_run': {
      const sql = (args.sql as string).trim();
      const config = connections.get(args.connectionId as string);
      if (!config || !open.has(config.id)) throw fail('connection', 'This connection is not open.');
      const lower = sql.toLowerCase();
      const options = args.options as RunOptions;
      history.unshift({
        id: history.length + 1,
        connectionId: config.id,
        sql,
        startedAt: Date.now(),
        elapsedMs: 12,
        rows: null,
        error: null,
      });
      if (
        config.environment === 'prod' &&
        /^(drop|truncate|alter)\b/.test(lower) &&
        !options.confirmed
      ) {
        throw fail('needs-confirmation', 'DROP removes objects permanently.');
      }
      if (lower.includes('sleep')) {
        const queryId = args.queryId as string;
        for (let i = 0; i < 100 && !cancelled.has(queryId); i++) {
          await new Promise((r) => setTimeout(r, 50));
        }
        if (cancelled.delete(queryId)) throw fail('cancelled', 'Query cancelled.');
      }
      if (lower.includes('lost_connection')) {
        // The server went away: the core's session is dead from here on.
        open.delete(config.id);
        throw fail('connection', 'The connection to the server was lost.');
      }
      const typo = lower.indexOf('fromm');
      if (typo >= 0) {
        throw fail('syntax', 'syntax error at or near "FROMM"', {
          code: '42601',
          position: typo + 1,
        });
      }
      const base: RunResult = {
        columns: [],
        rows: [],
        moreAvailable: false,
        rowsAffected: null,
        raw: null,
        notices: [],
        inTransaction: false,
        stoppedAtMemoryCap: false,
        editable: null,
        elapsedMs: 12,
        repeatable:
          /^(select|with|show|explain|values)\b/.test(lower) && !/\breturning\b/.test(lower),
      };
      if (options.refetch && !base.repeatable) {
        throw fail(
          'other',
          'This statement changes data, so Rowboat will not run it a second time.',
        );
      }
      if (!/^(select|with|show|explain|values)\b/.test(lower) && !/\breturning\b/.test(lower)) {
        return { ...base, rowsAffected: 1 } as T;
      }
      const limit = options.rowLimit || 5000;
      const total = 5000;
      const count = Math.min(limit, total);
      return {
        ...base,
        columns: [
          { name: 'id', typeName: 'int4', kind: 'number' },
          { name: 'name', typeName: 'text', kind: 'text' },
          { name: 'total', typeName: 'numeric', kind: 'number' },
          { name: 'note', typeName: 'text', kind: 'text' },
          { name: 'created_at', typeName: 'timestamptz', kind: 'temporal' },
        ],
        rows: Array.from({ length: count }, (_, i) => [
          String(i + 1),
          `Customer ${i + 1}`,
          ((i * 37) % 1000).toFixed(2),
          i % 7 === 0 ? null : `note ${i}`,
          `2026-11-${String((i % 28) + 1).padStart(2, '0')} 10:00:00+00`,
        ]),
        moreAvailable: count < total,
        editable: lower.includes('join')
          ? null
          : {
              schema: 'public',
              table: 'orders',
              columns: ['id', 'name', 'total', 'note', 'created_at'],
              key: ['id'],
            },
      } as T;
    }
    case 'update_check':
      // The fake "newer release" exists only on the beta channel.
      if (current.general.updateChecks === 'off' && !args.manual) return null as T;
      return (
        current.general.updateChannel === 'beta'
          ? {
              version: '0.2.0-beta.1',
              url: 'https://github.com/dominicplouffe/Rowboat/releases/tag/v0.2.0-beta.1',
              prerelease: true,
            }
          : null
      ) as T;
    case 'update_open':
      return undefined as T;
    case 'ai_key_set':
      if (args.key) aiKeys.add(args.providerId as string);
      else aiKeys.delete(args.providerId as string);
      return undefined as T;
    case 'ai_key_status':
      return Object.fromEntries(current.ai.providers.map((p) => [p.id, aiKeys.has(p.id)])) as T;
    case 'ai_models': {
      const provider = args.provider as AiProvider;
      requireKey(provider);
      return [`${provider.kind}-large`, `${provider.kind}-small`] as T;
    }
    case 'ai_test':
      requireKey(args.provider as AiProvider);
      return 'OK' as T;
    case 'ai_cancel':
      cancelled.add(args.id as string);
      return undefined as T;
    case 'ai_index':
      return {
        tables: 2,
        columns: 6,
        builtAt: Date.now(),
        fingerprint: 'fake',
        changed: Boolean(args.refresh),
        withSamples: false,
      } as T;
    case 'sample_database': {
      const existing = [...connections.values()].find((c) => c.name === 'Sample shop');
      if (existing) return view(existing) as T;
      const config: ConnectionConfig = {
        id: `fake-${nextId++}`,
        name: 'Sample shop',
        kind: 'sqlite',
        folder: '',
        color: '',
        environment: 'none',
        readOnly: false,
        host: '',
        port: 0,
        database: '',
        user: '',
        usePgpass: false,
        tls: { mode: 'disable', caCertPath: '', clientCertPath: '', clientKeyPath: '' },
        ssh: { enabled: false, host: '', port: 22, user: '', auth: 'agent', keyPath: '' },
        path: '(in memory)/sample.db',
        create: false,
        aiEnabled: true,
        aiSampleValues: false,
      };
      connections.set(config.id, config);
      return view(config) as T;
    }
    case 'ai_run':
      return fakeCopilot(
        args.request as AiRequest,
        args.onEvent as (event: AiEvent) => void,
      ) as Promise<T>;
    default:
      throw new Error(`unknown command: ${command}`);
  }
}

// ---- A pretend AI provider ----

const aiKeys = new Set<string>();

function aiFail(kind: string, message: string): { kind: string; message: string } {
  return { kind, message };
}

function requireKey(provider: AiProvider): void {
  if (!['local', 'custom'].includes(provider.kind) && !aiKeys.has(provider.id)) {
    throw aiFail('not-configured', `No API key is saved for ${provider.name || provider.kind}.`);
  }
}

async function fakeCopilot(
  request: AiRequest,
  onEvent: (event: AiEvent) => void,
): Promise<AiOutcome> {
  const provider = current.ai.providers.find((p) => p.id === current.ai.defaultProvider);
  if (!provider) {
    throw aiFail('not-configured', 'No AI provider is set up. Add one in Settings → AI.');
  }
  requireKey(provider);
  const connection = request.connectionId ? connections.get(request.connectionId) : undefined;
  if (connection && connection.aiEnabled === false) {
    throw aiFail('not-configured', `AI is turned off for the connection "${connection.name}".`);
  }
  if (request.task === 'fix' && !['syntax', 'schema'].includes(request.errorKind ?? '')) {
    throw aiFail('other', 'Only syntax and schema errors are sent to the AI.');
  }
  const model =
    request.task === 'fix' && provider.fixModel ? provider.fixModel : provider.model || 'fake';
  const user =
    request.task === 'fix'
      ? `This statement failed:\n${request.sql}\n\nThe database said:\n${request.error}`
      : request.task === 'explain'
        ? (request.sql ?? '')
        : (request.prompt ?? '');
  onEvent({
    type: 'context',
    provider: provider.name || provider.kind,
    request: {
      model,
      system:
        'You are the SQL copilot inside Rowboat.\n\n<schema>\ncustomers(id int pk, name text)\norders(id int pk, customer_id int fk→customers.id, total numeric, created_at timestamptz)\n</schema>',
      messages: [{ role: 'user', content: user }],
    },
    tablesSent: 2,
    tablesTotal: 2,
  });

  const prompt = (request.prompt ?? '').toLowerCase();
  let sql: string | null = null;
  let explanation: string;
  if (request.task === 'explain') {
    explanation = 'Returns every row of the query, newest first. Nothing is changed.';
  } else if (request.task === 'fix') {
    sql = (request.sql ?? '').replace(/fromm/gi, 'FROM');
    explanation = 'FROM was misspelled.';
  } else if (prompt.includes('delete')) {
    sql = 'DELETE FROM orders\nWHERE total = 0;';
    explanation = 'Removes orders with a zero total.';
  } else if (prompt.includes('nothing')) {
    explanation = 'There is no table for that in this schema.';
  } else {
    sql =
      'SELECT c.name, sum(o.total) AS revenue\nFROM orders o\nJOIN customers c ON c.id = o.customer_id\nGROUP BY c.name\nORDER BY revenue DESC;';
    explanation = 'Total order value per customer, highest first.';
  }
  const text = sql ? `\`\`\`sql\n${sql}\n\`\`\`\n${explanation}` : explanation;
  // Stream in a few pieces so the UI's streaming path is exercised.
  const slow = prompt.includes('slow');
  for (let i = 0; i < text.length; i += 24) {
    if (cancelled.delete(request.id)) throw aiFail('cancelled', 'Cancelled.');
    await new Promise((r) => setTimeout(r, slow ? 150 : 1));
    onEvent({ type: 'delta', text: text.slice(i, i + 24) });
  }
  if (cancelled.delete(request.id)) throw aiFail('cancelled', 'Cancelled.');
  return {
    text,
    sql,
    explanation,
    destructive: sql?.startsWith('DELETE') ? 'DELETE removes rows.' : null,
    parseError: null,
    usage: { inputTokens: 180, outputTokens: 42 },
    provider: provider.name || provider.kind,
    model,
  };
}
