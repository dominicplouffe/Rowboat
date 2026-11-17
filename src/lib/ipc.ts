// The only place the UI talks to the Rust core.
//
// In a plain browser (`npm run dev` without Tauri, or unit tests) there is no
// core, so development builds fall back to an in-memory stand-in. That branch
// is compiled out of production bundles.

import type { AppInfo, Settings } from './types';

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
};
