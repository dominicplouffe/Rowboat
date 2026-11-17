import type { Settings, Theme } from './types';

const CACHE_KEY = 'rowboat.theme';

/**
 * Applies the theme as early as possible, from a cached copy of the last
 * choice, so the window never flashes the wrong colors while settings load.
 * (The cache holds only "light" / "dark" / "system".)
 */
export function applyCachedTheme(): void {
  try {
    const cached = localStorage.getItem(CACHE_KEY);
    if (cached === 'light' || cached === 'dark') setTheme(cached);
  } catch {
    // Storage can be unavailable; the CSS falls back to the system theme.
  }
}

export function setTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === 'system') delete root.dataset.theme;
  else root.dataset.theme = theme;
  try {
    localStorage.setItem(CACHE_KEY, theme);
  } catch {
    // See above.
  }
  void syncNativeTheme(theme);
}

/** Keeps the native title bar in step with the chosen theme. */
async function syncNativeTheme(theme: Theme): Promise<void> {
  if (!('__TAURI_INTERNALS__' in window)) return;
  try {
    const { setTheme: setAppTheme } = await import('@tauri-apps/api/app');
    await setAppTheme(theme === 'system' ? null : theme);
  } catch {
    // Cosmetic only.
  }
}

/** Pushes appearance settings into CSS custom properties. */
export function applyAppearance(appearance: Settings['appearance']): void {
  setTheme(appearance.theme);
  const root = document.documentElement;
  root.dataset.density = appearance.density;
  const mono =
    'ui-monospace, "SF Mono", "Cascadia Mono", "Segoe UI Mono", Menlo, Consolas, monospace';
  const family = (name: string) => (name ? `${JSON.stringify(name)}, ${mono}` : mono);
  root.style.setProperty('--editor-font', family(appearance.editorFontFamily));
  root.style.setProperty('--editor-font-size', `${appearance.editorFontSize}px`);
  root.style.setProperty('--grid-font', family(appearance.gridFontFamily));
  root.style.setProperty('--grid-font-size', `${appearance.gridFontSize}px`);
}
