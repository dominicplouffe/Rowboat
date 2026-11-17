// Mirrors `src-tauri/src/settings.rs`. Keep the two in sync.

export type Theme = 'system' | 'light' | 'dark';

export interface Settings {
  version: number;
  general: {
    clientId: string;
    restoreTabs: boolean;
    updateChecks: 'auto' | 'notify' | 'off';
    updateChannel: 'stable' | 'beta';
    onboardingCompleted: boolean;
  };
  appearance: {
    theme: Theme;
    density: 'comfortable' | 'compact';
    editorFontFamily: string;
    editorFontSize: number;
    gridFontFamily: string;
    gridFontSize: number;
  };
  editor: {
    tabSize: number;
    indentWithTabs: boolean;
    wordWrap: boolean;
    lineNumbers: boolean;
    autoCloseBrackets: boolean;
    autocomplete: 'typing' | 'on-demand';
    formatterKeywordCase: 'upper' | 'lower' | 'preserve';
  };
  execution: {
    /** Rows per fetch; 0 means no limit. */
    defaultRowLimit: number;
    queryTimeoutSecs: number;
    autoCommit: boolean;
    stopScriptOnError: boolean;
    confirmDestructive: 'always' | 'prod-only' | 'never';
    resultMemoryCapMb: number;
  };
  results: {
    nullText: string;
    dateTimeFormat: string;
    timeZone: 'local' | 'utc';
    numberFormat: 'plain' | 'grouped';
    binaryDisplay: 'hex' | 'base64';
    maxCellChars: number;
    jsonAutoDetect: boolean;
  };
  connections: {
    connectTimeoutSecs: number;
    keepaliveSecs: number;
    autoReconnect: boolean;
    sshKnownHosts: 'ask' | 'strict' | 'accept-new';
  };
  ai: {
    autoSuggestFixes: boolean;
    sendSampleValues: boolean;
    schemaTokenBudget: number;
    requestTimeoutSecs: number;
  };
  keyboard: {
    overrides: Record<string, string>;
  };
  privacy: {
    keepHistory: boolean;
    historyRetentionDays: number;
  };
}

export type SectionId = Exclude<keyof Settings, 'version'>;

export interface AppInfo {
  name: string;
  version: string;
  os: string;
  arch: string;
  defaultClientId: string;
  settingsPath: string;
  /** False when there is no OS keychain: secrets last only until quit. */
  secretsPersistent: boolean;
}

// ---- Connections (mirrors src-tauri/src/connections.rs and manager.rs) ----

export type DbKind = 'postgres' | 'clickhouse' | 'sqlite';
export type Environment = 'none' | 'dev' | 'staging' | 'prod';
export type TlsMode = 'disable' | 'prefer' | 'require' | 'verify-ca' | 'verify-full';
export type SshAuth = 'agent' | 'password' | 'key';

export interface ConnectionConfig {
  id: string;
  name: string;
  kind: DbKind;
  folder: string;
  color: string;
  environment: Environment;
  readOnly: boolean;
  host: string;
  port: number;
  database: string;
  user: string;
  usePgpass: boolean;
  tls: { mode: TlsMode; caCertPath: string; clientCertPath: string; clientKeyPath: string };
  ssh: {
    enabled: boolean;
    host: string;
    port: number;
    user: string;
    auth: SshAuth;
    keyPath: string;
  };
  path: string;
  create: boolean;
}

/** A saved connection as the UI sees it: whether secrets exist, never their values. */
export interface ConnectionView extends ConnectionConfig {
  hasPassword: boolean;
  hasSshPassword: boolean;
  hasSshPassphrase: boolean;
  connected: boolean;
  serverVersion: string | null;
  supportsTransactions: boolean;
  inTransaction: boolean;
  /** This connection's mode; a new connection starts from the Settings default. */
  autoCommit: boolean;
}

/** Secrets typed by the user. Sent to the core once; never returned. */
export interface SecretInput {
  password?: string;
  sshPassword?: string;
  sshPassphrase?: string;
}

export type ErrorKind =
  | 'syntax'
  | 'schema'
  | 'permission'
  | 'connection'
  | 'cancelled'
  | 'timeout'
  | 'needs-confirmation'
  | 'unknown-host-key'
  | 'other';

export interface DbError {
  kind: ErrorKind;
  message: string;
  detail: string | null;
  hint: string | null;
  code: string | null;
  /** 1-based character offset into the statement. */
  position: number | null;
}

export type ColumnKind = 'text' | 'number' | 'bool' | 'temporal' | 'binary' | 'json' | 'other';

export interface Column {
  name: string;
  typeName: string;
  kind: ColumnKind;
}

export interface RunResult {
  columns: Column[];
  /** null is SQL NULL; everything else is display text. */
  rows: (string | null)[][];
  moreAvailable: boolean;
  rowsAffected: number | null;
  raw: string | null;
  notices: string[];
  inTransaction: boolean;
  elapsedMs: number;
}

export interface SchemaNode {
  name: string;
  kind: string;
  detail: string | null;
  hasChildren: boolean;
}

export interface StatementSpan {
  start: number;
  end: number;
}

export type FilePurpose = 'sqlite-open' | 'sqlite-new' | 'certificate' | 'key';
