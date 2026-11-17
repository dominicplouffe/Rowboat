// Keyboard shortcuts: every action has a default chord and can be rebound in
// Settings → Keyboard. Chords are strings like "Mod+Shift+Enter", where "Mod"
// is ⌘ on macOS and Ctrl elsewhere.

export interface Action {
  id: string;
  label: string;
  group: string;
  /** Default chord. */
  chord: string;
  run: () => void;
}

export const isMac =
  typeof navigator !== 'undefined' &&
  /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

const MODIFIER_KEYS = new Set(['Control', 'Meta', 'Alt', 'Shift', 'AltGraph', 'CapsLock']);

/** The chord a key event represents, or null for a bare modifier press. */
export function chordFromEvent(event: KeyboardEvent, mac = isMac): string | null {
  if (MODIFIER_KEYS.has(event.key)) return null;
  const parts: string[] = [];
  if (mac ? event.metaKey : event.ctrlKey) parts.push('Mod');
  if (mac && event.ctrlKey) parts.push('Ctrl');
  if (!mac && event.metaKey) parts.push('Meta');
  if (event.altKey) parts.push('Alt');
  if (event.shiftKey) parts.push('Shift');
  parts.push(keyName(event));
  return parts.join('+');
}

function keyName(event: KeyboardEvent): string {
  // Prefer the physical key for letters and digits so Shift/Alt do not change
  // the name ("Mod+Shift+P", not "Mod+Shift+p"; "Alt+1", not "Alt+¡").
  const code = event.code ?? '';
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (event.key === ' ') return 'Space';
  return event.key.length === 1 ? event.key.toUpperCase() : event.key;
}

/** A chord is bindable only if it cannot be produced by ordinary typing. */
export function isBindable(chord: string): boolean {
  const parts = chord.split('+');
  const key = parts[parts.length - 1];
  const hasCommandModifier = parts.some(
    (p) => p === 'Mod' || p === 'Ctrl' || p === 'Meta' || p === 'Alt',
  );
  return hasCommandModifier || /^F\d{1,2}$/.test(key);
}

const SYMBOLS_MAC: Record<string, string> = {
  Mod: '⌘',
  Ctrl: '⌃',
  Alt: '⌥',
  Shift: '⇧',
  Enter: '↩',
  Backspace: '⌫',
  Escape: 'Esc',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
};
const SYMBOLS_OTHER: Record<string, string> = {
  Mod: 'Ctrl',
  Meta: 'Win',
  Escape: 'Esc',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
};

/** Human-readable chord: "⌘⇧P" on macOS, "Ctrl+Shift+P" elsewhere. */
export function formatChord(chord: string, mac = isMac): string {
  const parts = chord.split('+').map((p) => (mac ? SYMBOLS_MAC[p] : SYMBOLS_OTHER[p]) ?? p);
  return mac ? parts.join('') : parts.join('+');
}

/** The chord currently bound to `action`, honoring user overrides. */
export function chordFor(action: Action, overrides: Record<string, string>): string {
  return overrides[action.id] ?? action.chord;
}

/** Finds the action a chord triggers, honoring user overrides. */
export function findAction(
  actions: readonly Action[],
  overrides: Record<string, string>,
  chord: string,
): Action | undefined {
  return actions.find((action) => chordFor(action, overrides) === chord);
}
