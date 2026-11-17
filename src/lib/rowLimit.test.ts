import { describe, expect, it } from 'vitest';
import { formatRowLimit, NO_LIMIT, parseRowLimit, ROW_LIMIT_PRESETS } from './rowLimit';

describe('row limit', () => {
  it('offers the presets from the spec, with 1,000 among them', () => {
    expect([...ROW_LIMIT_PRESETS]).toEqual([100, 500, 1000, 5000]);
  });

  it('formats limits for the toolbar', () => {
    expect(formatRowLimit(1000)).toBe('1,000 rows');
    expect(formatRowLimit(NO_LIMIT)).toBe('No limit');
  });

  it('parses custom limits, tolerating separators', () => {
    expect(parseRowLimit('250')).toBe(250);
    expect(parseRowLimit(' 25,000 ')).toBe(25000);
    expect(parseRowLimit('10_000')).toBe(10000);
  });

  it('rejects anything that is not a positive row count', () => {
    for (const bad of ['', '0', '-5', '1.5', 'abc', '1e3', '99999999']) {
      expect(parseRowLimit(bad), bad).toBeNull();
    }
  });
});
