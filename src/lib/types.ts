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
}
