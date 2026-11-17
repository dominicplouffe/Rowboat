// In-memory stand-in for the Rust core, for browser-only development and
// tests. Never part of a production bundle (see ipc.ts).

import type { AppInfo, Settings } from './types';

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
      throw new Error(`unknown command: ${command}`);
  }
}
