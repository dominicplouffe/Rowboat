import { describe, expect, it } from 'vitest';
import {
  chordFor,
  chordFromEvent,
  findAction,
  formatChord,
  isBindable,
  type Action,
} from './shortcuts';

const key = (init: KeyboardEventInit) => new KeyboardEvent('keydown', init);

describe('chordFromEvent', () => {
  it('maps ⌘ on macOS and Ctrl elsewhere to Mod', () => {
    expect(chordFromEvent(key({ key: ',', code: 'Comma', metaKey: true }), true)).toBe('Mod+,');
    expect(chordFromEvent(key({ key: ',', code: 'Comma', ctrlKey: true }), false)).toBe('Mod+,');
  });

  it('keeps Ctrl distinct from ⌘ on macOS', () => {
    expect(chordFromEvent(key({ key: 'Tab', code: 'Tab', ctrlKey: true }), true)).toBe('Ctrl+Tab');
  });

  it('names letters by physical key, whatever Shift or Alt produce', () => {
    const event = key({ key: 'π', code: 'KeyP', metaKey: true, altKey: true, shiftKey: true });
    expect(chordFromEvent(event, true)).toBe('Mod+Alt+Shift+P');
  });

  it('orders modifiers consistently', () => {
    const event = key({ key: 'Enter', code: 'Enter', ctrlKey: true, shiftKey: true });
    expect(chordFromEvent(event, false)).toBe('Mod+Shift+Enter');
  });

  it('ignores bare modifier presses', () => {
    expect(chordFromEvent(key({ key: 'Shift', shiftKey: true }), false)).toBeNull();
    expect(chordFromEvent(key({ key: 'Meta', metaKey: true }), true)).toBeNull();
  });
});

describe('isBindable', () => {
  it('rejects chords that ordinary typing produces', () => {
    expect(isBindable('A')).toBe(false);
    expect(isBindable('Shift+A')).toBe(false);
    expect(isBindable('Enter')).toBe(false);
  });

  it('accepts command chords and function keys', () => {
    expect(isBindable('Mod+K')).toBe(true);
    expect(isBindable('Alt+ArrowLeft')).toBe(true);
    expect(isBindable('F5')).toBe(true);
    expect(isBindable('Shift+F12')).toBe(true);
  });
});

describe('formatChord', () => {
  it('uses symbols on macOS', () => {
    expect(formatChord('Mod+Shift+Enter', true)).toBe('⌘⇧↩');
    expect(formatChord('Mod+,', true)).toBe('⌘,');
  });

  it('uses words elsewhere', () => {
    expect(formatChord('Mod+Shift+Enter', false)).toBe('Ctrl+Shift+Enter');
  });
});

describe('overrides', () => {
  const actions: Action[] = [
    { id: 'a', label: 'A', group: 'g', chord: 'Mod+A', run: () => {} },
    { id: 'b', label: 'B', group: 'g', chord: 'Mod+B', run: () => {} },
  ];

  it('uses the default chord until an action is rebound', () => {
    expect(chordFor(actions[0], {})).toBe('Mod+A');
    expect(chordFor(actions[0], { a: 'Mod+Shift+A' })).toBe('Mod+Shift+A');
  });

  it('a rebound action no longer answers to its default chord', () => {
    const overrides = { a: 'Mod+Shift+A' };
    expect(findAction(actions, overrides, 'Mod+A')).toBeUndefined();
    expect(findAction(actions, overrides, 'Mod+Shift+A')?.id).toBe('a');
    expect(findAction(actions, overrides, 'Mod+B')?.id).toBe('b');
  });
});
