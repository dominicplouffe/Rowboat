import { describe, expect, it } from 'vitest';
import { compareCells, formatCell, looksLikeJson, viewRows } from './cells';
import { defaultSettings } from './devBackend';
import type { Column, ColumnKind } from './types';

const column = (kind: ColumnKind, typeName = ''): Column => ({ name: 'c', typeName, kind });
const base = defaultSettings().results;

describe('formatCell', () => {
  it('groups digits only when asked, and only for plain numbers', () => {
    const grouped = { ...base, numberFormat: 'grouped' as const };
    expect(formatCell('1234567.891', column('number'), base)).toBe('1234567.891');
    expect(formatCell('1234567.891', column('number'), grouped)).toBe('1,234,567.891');
    expect(formatCell('-1000', column('number'), grouped)).toBe('-1,000');
    expect(formatCell('1e+20', column('number'), grouped)).toBe('1e+20');
    expect(formatCell('1234567', column('text'), grouped)).toBe('1234567');
  });

  it('shows instants in the chosen time zone', () => {
    const utc = { ...base, timeZone: 'utc' as const };
    const tz = column('temporal', 'timestamptz');
    expect(formatCell('2024-02-29 13:14:15.5+00', tz, utc)).toBe('2024-02-29 13:14:15.5');
    expect(formatCell('2024-02-29 13:14:15+02', tz, utc)).toBe('2024-02-29 11:14:15');
    // Local time: whatever this machine's zone is, it must be the same instant.
    const local = formatCell('2024-02-29 13:14:15+00', tz, {
      ...base,
      timeZone: 'local',
      dateTimeFormat: 'YYYY-MM-DDTHH:mm:ssZ',
    });
    expect(new Date(local).getTime()).toBe(Date.UTC(2024, 1, 29, 13, 14, 15));
    // ClickHouse DateTime values are instants too.
    expect(formatCell('2024-02-29 13:14:15', column('temporal', "DateTime('UTC')"), utc)).toBe(
      '2024-02-29 13:14:15',
    );
  });

  it('never shifts wall-clock timestamps or plain dates', () => {
    const local = { ...base, timeZone: 'local' as const };
    expect(formatCell('2024-02-29 13:14:15', column('temporal', 'timestamp'), local)).toBe(
      '2024-02-29 13:14:15',
    );
    expect(formatCell('2024-02-29', column('temporal', 'date'), local)).toBe('2024-02-29');
    expect(formatCell('infinity', column('temporal', 'timestamptz'), local)).toBe('infinity');
  });

  it('applies the date/time format', () => {
    const custom = { ...base, timeZone: 'utc' as const, dateTimeFormat: 'DD/MM/YYYY HH:mm' };
    expect(formatCell('2024-02-29 13:14:15+00', column('temporal', 'timestamptz'), custom)).toBe(
      '29/02/2024 13:14',
    );
  });

  it('shows binary as hex or base64', () => {
    expect(formatCell('\\xdeadbeef', column('binary'), base)).toBe('\\xdeadbeef');
    expect(formatCell('\\xdeadbeef', column('binary'), { ...base, binaryDisplay: 'base64' })).toBe(
      '3q2+7w==',
    );
  });

  it('cuts long values at the configured length', () => {
    const short = { ...base, maxCellChars: 20 };
    expect(formatCell('x'.repeat(50), column('text'), short)).toBe(`${'x'.repeat(20)}…`);
    expect(formatCell('short', column('text'), short)).toBe('short');
  });
});

describe('looksLikeJson', () => {
  it('recognises JSON columns and JSON-looking text', () => {
    expect(looksLikeJson('{"a":1}', column('json'), false)).toBe(true);
    expect(looksLikeJson(' [1, 2] ', column('text'), true)).toBe(true);
    expect(looksLikeJson('{"a":1}', column('text'), false)).toBe(false);
    expect(looksLikeJson('{not json}', column('text'), true)).toBe(false);
    expect(looksLikeJson('plain', column('text'), true)).toBe(false);
    expect(looksLikeJson('123', column('text'), true)).toBe(false);
    expect(looksLikeJson(null, column('json'), true)).toBe(false);
  });
});

describe('sorting and filtering', () => {
  const columns = [column('number'), column('text')];
  const rows = [
    ['10', 'pear'],
    ['9', 'Apple'],
    [null, 'fig'],
    ['100', null],
    ['9', 'banana'],
  ];
  const sorted = (view: Parameters<typeof viewRows>[2]) =>
    viewRows(rows, columns, view).map((i) => rows[i][0]);

  it('sorts numbers by value, not as text, with NULLs last both ways', () => {
    expect(sorted({ sort: { column: 0, direction: 'asc' }, filter: '' })).toEqual([
      '9',
      '9',
      '10',
      '100',
      null,
    ]);
    expect(sorted({ sort: { column: 0, direction: 'desc' }, filter: '' })).toEqual([
      '100',
      '10',
      '9',
      '9',
      null,
    ]);
  });

  it('keeps fetched order among equal values', () => {
    const order = viewRows(rows, columns, { sort: { column: 0, direction: 'asc' }, filter: '' });
    expect(order.slice(0, 2)).toEqual([1, 4]);
  });

  it('filters across columns, ignoring case, before sorting', () => {
    expect(viewRows(rows, columns, { sort: null, filter: 'AP' })).toEqual([1]);
    expect(viewRows(rows, columns, { sort: null, filter: '9' })).toEqual([1, 4]);
    expect(viewRows(rows, columns, { sort: null, filter: 'zzz' })).toEqual([]);
    expect(viewRows(rows, columns, { sort: null, filter: '  ' })).toEqual([0, 1, 2, 3, 4]);
  });

  it('compares text plainly', () => {
    expect(compareCells('a', 'b', column('text'), 'asc')).toBeLessThan(0);
    expect(compareCells('a', 'b', column('text'), 'desc')).toBeGreaterThan(0);
    expect(compareCells(null, null, column('text'), 'asc')).toBe(0);
  });
});
