// Describes the Settings window: which fields exist, how each is edited, and
// the words that find it in search. The values themselves live in the Rust
// core (src-tauri/src/settings.rs).

import type { SectionId, Settings } from './types';

type Option = { value: string; label: string };

/** A (section, key) pair that really exists in `Settings`. */
type FieldRef = {
  [S in SectionId]: { section: S; key: keyof Settings[S] & string };
}[SectionId];

type FieldBase = FieldRef & {
  label: string;
  help?: string;
  /** Extra search terms. */
  keywords?: string;
  /**
   * Set while nothing in the app reads this setting yet: the milestone that
   * makes it work. Such fields are shown disabled, with the milestone named.
   */
  arrives?: Milestone;
};

export type Milestone = 'M1' | 'M2' | 'M3' | 'M4';

export const MILESTONE_NAMES: Record<Milestone, string> = {
  M1: 'connections',
  M2: 'the editor and results grid',
  M3: 'the AI copilot',
  M4: 'the first release',
};

export type Field =
  | (FieldBase & { kind: 'toggle' })
  | (FieldBase & { kind: 'select'; options: Option[] })
  | (FieldBase & { kind: 'number'; min: number; max: number; step?: number; unit?: string })
  | (FieldBase & { kind: 'text'; placeholder?: string; maxLength?: number });

export interface Section {
  id: SectionId;
  title: string;
  blurb: string;
}

export const SECTIONS: Section[] = [
  { id: 'general', title: 'General', blurb: 'Identity, startup and updates.' },
  { id: 'appearance', title: 'Appearance', blurb: 'Theme, density and fonts.' },
  { id: 'editor', title: 'Editor', blurb: 'How the SQL editor behaves.' },
  { id: 'execution', title: 'Execution', blurb: 'Row limits, timeouts and safety.' },
  { id: 'results', title: 'Results', blurb: 'How values are shown in the grid.' },
  { id: 'connections', title: 'Connections', blurb: 'Timeouts, keepalive and SSH.' },
  { id: 'ai', title: 'AI', blurb: 'The copilot never runs SQL itself.' },
  { id: 'keyboard', title: 'Keyboard', blurb: 'View and rebind every shortcut.' },
  { id: 'privacy', title: 'Privacy & data', blurb: 'History and your settings file.' },
];

const opts = (...pairs: [string, string][]): Option[] =>
  pairs.map(([value, label]) => ({ value, label }));

export const FIELDS: Field[] = [
  // General
  {
    section: 'general',
    key: 'clientId',
    kind: 'text',
    label: 'Client ID',
    help: 'Identifies your sessions to DBAs: sent as application_name on PostgreSQL and as the client name on ClickHouse.',
    keywords: 'application_name identity session name',
    maxLength: 63,
  },
  {
    section: 'general',
    key: 'restoreTabs',
    kind: 'toggle',
    label: 'Restore tabs on start',
    help: 'Reopen the tabs and unsaved text from your last session.',
    keywords: 'session restore startup',
  },
  {
    section: 'general',
    key: 'updateChecks',
    kind: 'select',
    label: 'Update checks',
    options: opts(['auto', 'Install automatically'], ['notify', 'Notify me'], ['off', 'Off']),
    help: 'Rowboat asks GitHub which releases exist and sends nothing else. For now both choices show a notice in the status bar that opens the download page: installing by itself needs signed builds, which are not set up yet. Off makes no request.',
    keywords: 'auto-update upgrade new version',
  },
  {
    section: 'general',
    key: 'updateChannel',
    kind: 'select',
    label: 'Update channel',
    options: opts(['stable', 'Stable'], ['beta', 'Beta']),
    help: 'Beta also offers pre-releases.',
    keywords: 'release prerelease',
  },

  // Appearance
  {
    section: 'appearance',
    key: 'theme',
    kind: 'select',
    label: 'Theme',
    options: opts(['system', 'System'], ['light', 'Light'], ['dark', 'Dark']),
    keywords: 'dark mode light mode colors',
  },
  {
    section: 'appearance',
    key: 'density',
    kind: 'select',
    label: 'UI density',
    options: opts(['comfortable', 'Comfortable'], ['compact', 'Compact']),
    keywords: 'spacing size',
  },
  {
    section: 'appearance',
    key: 'editorFontFamily',
    kind: 'text',
    label: 'Editor font',
    placeholder: 'System monospace',
    help: 'Any font installed on this computer. Leave empty for the system monospace font.',
    keywords: 'typeface family monospace',
  },
  {
    section: 'appearance',
    key: 'editorFontSize',
    kind: 'number',
    label: 'Editor font size',
    min: 8,
    max: 32,
    unit: 'px',
  },
  {
    section: 'appearance',
    key: 'gridFontFamily',
    kind: 'text',
    label: 'Grid font',
    placeholder: 'System monospace',
    keywords: 'typeface family results',
  },
  {
    section: 'appearance',
    key: 'gridFontSize',
    kind: 'number',
    label: 'Grid font size',
    min: 8,
    max: 32,
    unit: 'px',
  },

  // Editor
  {
    section: 'editor',
    key: 'tabSize',
    kind: 'number',
    label: 'Tab size',
    min: 1,
    max: 8,
    unit: 'spaces',
    keywords: 'indent width',
  },
  {
    section: 'editor',
    key: 'indentWithTabs',
    kind: 'toggle',
    label: 'Indent with tabs',
    help: 'Off: indent with spaces.',
    keywords: 'spaces vs tabs',
  },
  {
    section: 'editor',
    key: 'wordWrap',
    kind: 'toggle',
    label: 'Word wrap',
    keywords: 'line wrapping soft wrap',
  },
  {
    section: 'editor',
    key: 'lineNumbers',
    kind: 'toggle',
    label: 'Line numbers',
    keywords: 'gutter',
  },
  {
    section: 'editor',
    key: 'autoCloseBrackets',
    kind: 'toggle',
    label: 'Auto-close brackets and quotes',
    keywords: 'pairs parentheses',
  },
  {
    section: 'editor',
    key: 'autocomplete',
    kind: 'select',
    label: 'Autocomplete',
    options: opts(['typing', 'As you type'], ['on-demand', 'On demand (Ctrl+Space)']),
    keywords: 'completion suggestions intellisense',
  },
  {
    section: 'editor',
    key: 'formatterKeywordCase',
    kind: 'select',
    label: 'Formatter keyword case',
    options: opts(['upper', 'UPPERCASE'], ['lower', 'lowercase'], ['preserve', 'Leave as typed']),
    keywords: 'format sql beautify',
  },

  // Execution
  {
    section: 'execution',
    key: 'defaultRowLimit',
    kind: 'number',
    label: 'Default row limit',
    min: 0,
    max: 1_000_000,
    step: 100,
    unit: 'rows',
    help: 'Rows fetched per page in new tabs; 0 means no limit. Applied when fetching — your SQL is never rewritten. Change it per tab from the results toolbar.',
    keywords: 'limit rows fetch page',
  },
  {
    section: 'execution',
    key: 'queryTimeoutSecs',
    kind: 'number',
    label: 'Query timeout',
    min: 0,
    max: 86_400,
    unit: 'seconds',
    help: '0 means no timeout.',
    keywords: 'statement timeout cancel',
  },
  {
    section: 'execution',
    key: 'autoCommit',
    kind: 'toggle',
    label: 'Auto-commit by default',
    help: 'How PostgreSQL and SQLite connections start out. Each connection can be switched from its toolbar.',
    keywords: 'transaction commit rollback',
  },
  {
    section: 'execution',
    key: 'stopScriptOnError',
    kind: 'toggle',
    label: 'Stop script on error',
    keywords: 'run script continue',
  },
  {
    section: 'execution',
    key: 'confirmDestructive',
    kind: 'select',
    label: 'Confirm destructive statements',
    options: opts(['always', 'Always'], ['prod-only', 'Prod connections only'], ['never', 'Never']),
    help: 'DROP, TRUNCATE, ALTER, and UPDATE or DELETE without WHERE.',
    keywords: 'prod guard drop truncate delete safety',
  },
  {
    section: 'execution',
    key: 'resultMemoryCapMb',
    kind: 'number',
    label: 'Result memory cap',
    min: 16,
    max: 16_384,
    step: 16,
    unit: 'MB',
    help: 'Fetch all warns before a result grows past this.',
    keywords: 'memory limit fetch all',
  },

  // Results
  {
    section: 'results',
    key: 'nullText',
    kind: 'text',
    label: 'NULL display text',
    maxLength: 32,
    keywords: 'null empty',
  },
  {
    section: 'results',
    key: 'dateTimeFormat',
    kind: 'text',
    label: 'Date/time format',
    placeholder: 'YYYY-MM-DD HH:mm:ss',
    maxLength: 64,
    keywords: 'timestamp date format',
  },
  {
    section: 'results',
    key: 'timeZone',
    kind: 'select',
    label: 'Time zone',
    options: opts(['local', 'Local'], ['utc', 'UTC']),
    keywords: 'timezone timestamps',
  },
  {
    section: 'results',
    key: 'numberFormat',
    kind: 'select',
    label: 'Number format',
    options: opts(['plain', '1234567.89'], ['grouped', '1,234,567.89']),
    keywords: 'thousands separator digits',
  },
  {
    section: 'results',
    key: 'binaryDisplay',
    kind: 'select',
    label: 'Binary display',
    options: opts(['hex', 'Hex'], ['base64', 'Base64']),
    keywords: 'bytea blob bytes',
  },
  {
    section: 'results',
    key: 'maxCellChars',
    kind: 'number',
    label: 'Max characters per cell',
    min: 20,
    max: 100_000,
    step: 10,
    unit: 'chars',
    keywords: 'truncate long text',
  },
  {
    section: 'results',
    key: 'jsonAutoDetect',
    kind: 'toggle',
    label: 'Detect JSON in text cells',
    help: 'Open the JSON viewer for any text cell that parses as JSON.',
    keywords: 'json viewer',
  },

  // Connections
  {
    section: 'connections',
    key: 'connectTimeoutSecs',
    kind: 'number',
    label: 'Connect timeout',
    min: 1,
    max: 600,
    unit: 'seconds',
  },
  {
    section: 'connections',
    key: 'keepaliveSecs',
    kind: 'number',
    label: 'Keepalive interval',
    min: 0,
    max: 3600,
    unit: 'seconds',
    help: '0 turns keepalive off.',
    keywords: 'keep-alive ping idle',
  },
  {
    section: 'connections',
    key: 'autoReconnect',
    kind: 'toggle',
    label: 'Reconnect automatically',
    help: 'Tabs keep their state.',
    keywords: 'auto-reconnect dropped',
  },
  {
    section: 'connections',
    key: 'sshKnownHosts',
    kind: 'select',
    label: 'SSH known_hosts policy',
    options: opts(
      ['ask', 'Ask about unknown hosts'],
      ['strict', 'Strict: refuse unknown hosts'],
      ['accept-new', 'Accept new hosts'],
    ),
    keywords: 'ssh tunnel host key fingerprint',
  },

  // AI
  {
    section: 'ai',
    key: 'autoSuggestFixes',
    kind: 'toggle',
    label: 'Suggest fixes automatically',
    help: 'When a query fails with a syntax or schema error, show a fix as a diff to accept or dismiss. The request includes your query text, which may contain literals. Off: a button under the error asks for one.',
    keywords: 'fix my query error diff',
  },
  {
    section: 'ai',
    key: 'sendSampleValues',
    kind: 'toggle',
    label: 'Send sample values (default for new connections)',
    help: 'A few common values of low-cardinality columns help the AI write WHERE filters. Each connection has its own switch; this is what new connections start with. Off: only schema metadata is sent. PostgreSQL only for now.',
    keywords: 'privacy row data samples',
  },
  {
    section: 'ai',
    key: 'schemaTokenBudget',
    kind: 'number',
    label: 'Schema token budget',
    min: 500,
    max: 200_000,
    step: 500,
    unit: 'tokens',
    help: 'How much schema context a request may include.',
    keywords: 'context size prompt',
  },
  {
    section: 'ai',
    key: 'requestTimeoutSecs',
    kind: 'number',
    label: 'Request timeout',
    min: 5,
    max: 600,
    unit: 'seconds',
    help: 'How long the provider may stay silent before a request is given up. An answer that keeps arriving is never cut off.',
    keywords: 'llm provider',
  },

  // Privacy & data
  {
    section: 'privacy',
    key: 'keepHistory',
    kind: 'toggle',
    label: 'Keep query history',
    keywords: 'history log',
  },
  {
    section: 'privacy',
    key: 'historyRetentionDays',
    kind: 'number',
    label: 'History retention',
    min: 1,
    max: 3650,
    unit: 'days',
    keywords: 'history retention delete',
  },
];

/**
 * Parses what was typed into a number field. Returns null for empty or
 * non-numeric text, so that clearing a field can never be read as 0 (which
 * means "no limit" or "off" for several settings).
 */
export function parseNumberField(text: string, min: number, max: number): number | null {
  const trimmed = text.trim();
  if (trimmed === '') return null;
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed)) return null;
  return Math.min(max, Math.max(min, Math.round(parsed)));
}

/** Case-insensitive match of every search word against a field's text. */
export function matchesQuery(field: Field, sectionTitle: string, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;
  const haystack = [field.label, field.help, field.keywords, sectionTitle, field.key]
    .filter(Boolean)
    .join(' ')
    .toLowerCase();
  return words.every((word) => haystack.includes(word));
}
