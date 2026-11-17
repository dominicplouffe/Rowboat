// The grid's performance budget: scrolling stays smooth with 100k rows loaded
// because only the rows in view exist in the DOM. Frame rate cannot be
// measured without a real display, so this checks the property it rests on.

import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import Grid from './Grid.svelte';

const ROWS = 100_000;
const columns = [
  { name: 'id', typeName: 'int4', kind: 'number' as const },
  { name: 'name', typeName: 'text', kind: 'text' as const },
  { name: 'note', typeName: 'text', kind: 'text' as const },
];
const rows = Array.from({ length: ROWS }, (_, i) => [String(i + 1), `Customer ${i + 1}`, null]);

let grid: ReturnType<typeof mount> | undefined;

beforeAll(() => {
  // jsdom has no layout engine; the grid only needs these to exist.
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

afterEach(() => {
  if (grid) void unmount(grid);
  grid = undefined;
  document.body.innerHTML = '';
});

function render() {
  grid = mount(Grid, { target: document.body, props: { columns, rows } });
  flushSync();
  return document.querySelector<HTMLElement>('.grid')!;
}

const rowCount = () => document.querySelectorAll('.row').length;
const firstRowNumber = () =>
  Number(document.querySelector('.row')?.textContent?.trim().split(/\s+/)[0]);

describe('grid with 100,000 rows', () => {
  it('puts only the rows in view into the DOM', () => {
    const started = performance.now();
    render();
    const elapsed = performance.now() - started;
    expect(rowCount()).toBeGreaterThan(0);
    expect(rowCount()).toBeLessThan(100);
    // Mounting must not walk every row: generous, to stay stable on slow CI.
    expect(elapsed).toBeLessThan(1500);
  });

  it('scrolling swaps rows instead of adding them', () => {
    const viewport = render();
    expect(firstRowNumber()).toBe(1);

    viewport.scrollTop = 24 * 50_000;
    viewport.dispatchEvent(new Event('scroll'));
    flushSync();
    expect(rowCount()).toBeLessThan(100);
    expect(firstRowNumber()).toBeGreaterThan(49_900);
    expect(firstRowNumber()).toBeLessThanOrEqual(50_001);

    viewport.scrollTop = 24 * ROWS;
    viewport.dispatchEvent(new Event('scroll'));
    flushSync();
    expect(rowCount()).toBeLessThan(100);
    expect(document.body.textContent).toContain(`Customer ${ROWS}`);
  });

  it('the scroll area is as tall as all the rows, so the scrollbar is honest', () => {
    render();
    const tall = [...document.querySelectorAll<HTMLElement>('.grid *')].some(
      (el) => parseFloat(el.style.height) >= 24 * ROWS,
    );
    expect(tall).toBe(true);
  });
});
