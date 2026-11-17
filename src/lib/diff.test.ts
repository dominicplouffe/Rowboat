import { describe, expect, it } from 'vitest';
import { changedSpan, diffLines } from './diff';

describe('diffLines', () => {
  it('marks only the line that changed', () => {
    expect(
      diffLines('SELECT id\nFROMM orders\nWHERE x = 1', 'SELECT id\nFROM orders\nWHERE x = 1'),
    ).toEqual([
      { kind: 'same', text: 'SELECT id' },
      { kind: 'removed', text: 'FROMM orders' },
      { kind: 'added', text: 'FROM orders' },
      { kind: 'same', text: 'WHERE x = 1' },
    ]);
  });

  it('handles added and removed lines and Windows line endings', () => {
    expect(diffLines('a\r\nb', 'a\nb\nc').map((l) => `${l.kind}:${l.text}`)).toEqual([
      'same:a',
      'same:b',
      'added:c',
    ]);
    expect(diffLines('a\nb\nc', 'a\nc').map((l) => l.kind)).toEqual(['same', 'removed', 'same']);
  });

  it('identical text has no changes', () => {
    expect(diffLines('SELECT 1', 'SELECT 1').every((l) => l.kind === 'same')).toBe(true);
  });
});

describe('changedSpan', () => {
  it('finds the part of a line that differs', () => {
    const { before, after } = changedSpan('SELECT totl FROM orders', 'SELECT total FROM orders');
    expect(before.filter((s) => s.changed)).toEqual([]);
    expect(after.filter((s) => s.changed).map((s) => s.text)).toEqual(['a']);
    expect(after.map((s) => s.text).join('')).toBe('SELECT total FROM orders');
  });

  it('a wholly different line is all changed', () => {
    const { before, after } = changedSpan('abc', 'xyz');
    expect(before).toEqual([{ text: 'abc', changed: true }]);
    expect(after).toEqual([{ text: 'xyz', changed: true }]);
  });
});
