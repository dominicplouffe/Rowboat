<script lang="ts">
  // Results grid. Only the rows in view are in the DOM, so scrolling stays
  // smooth however many rows are loaded. Sorting and filtering reorder row
  // indexes; the rows themselves are never copied.
  import { formatCell, viewRows, type GridView } from '../lib/cells';
  import { settings } from '../lib/settings.svelte';
  import type { Column, EditTarget } from '../lib/types';

  let {
    columns,
    rows,
    filter = '',
    hidden = [],
    editable = null,
    edits = {},
    selected = $bindable(null),
    onedit,
    onshown,
  }: {
    columns: Column[];
    rows: (string | null)[][];
    filter?: string;
    /** Indexes of hidden columns. */
    hidden?: number[];
    editable?: EditTarget | null;
    /** Staged edits: row index → column index → value. */
    edits?: Record<number, Record<number, string | null>>;
    selected?: { row: number; column: number } | null;
    onedit?: (row: number, column: number, value: string | null) => void;
    /** Reports how many rows pass the filter. */
    onshown?: (count: number) => void;
  } = $props();

  const ROW = 24;
  const HEADER = 40;
  const OVERSCAN = 8;
  const CHAR = 7.4; // average glyph width of the grid font at 12px

  let viewport = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let height = $state(300);
  let sort = $state<GridView['sort']>(null);
  let resized = $state<Record<number, number>>({});
  let editing = $state<{ row: number; column: number; text: string } | null>(null);
  let copied = $state(false);

  const results = $derived(settings.value?.results);
  const nullText = $derived(results?.nullText ?? 'NULL');
  const fontSize = $derived(settings.value?.appearance.gridFontSize ?? 12);

  const shown = $derived(columns.map((_, i) => i).filter((i) => !hidden.includes(i)));
  const order = $derived(viewRows(rows, columns, { sort, filter }));
  $effect(() => onshown?.(order.length));

  // Column widths from the header and a sample of the data, unless resized.
  const widths = $derived.by(() => {
    const scale = fontSize / 12;
    return columns.map((column, c) => {
      if (resized[c]) return resized[c];
      let longest = Math.max(column.name.length + 2, column.typeName.length * 0.85);
      for (let r = 0; r < Math.min(rows.length, 200); r++) {
        longest = Math.max(longest, Math.min(rows[r][c]?.length ?? nullText.length, 60));
      }
      return Math.round(Math.min(420, Math.max(64, longest * CHAR * scale + 22)));
    });
  });
  const gutter = $derived(Math.max(44, String(rows.length).length * 8 + 20));
  const template = $derived(`${gutter}px ${shown.map((c) => `${widths[c]}px`).join(' ')}`);
  const totalWidth = $derived(gutter + shown.reduce((sum, c) => sum + widths[c], 0));

  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN));
  const last = $derived(Math.min(order.length, Math.ceil((scrollTop + height) / ROW) + OVERSCAN));
  const visible = $derived(order.slice(first, last));

  // A new result starts at the top with nothing selected or sorted.
  let previousRows: unknown = null;
  $effect(() => {
    if (rows === previousRows) return;
    // More rows of the same query (next page) keep the view; a new result resets it.
    const samePrefix =
      Array.isArray(previousRows) && previousRows.length > 0 && rows[0] === previousRows[0];
    previousRows = rows;
    if (samePrefix) return;
    selected = null;
    editing = null;
    if (viewport) viewport.scrollTop = 0;
  });
  $effect(() => {
    void columns;
    sort = null;
    resized = {};
  });

  $effect(() => {
    if (!viewport) return;
    const observer = new ResizeObserver(() => (height = viewport!.clientHeight));
    observer.observe(viewport);
    return () => observer.disconnect();
  });

  const canEdit = (column: number) =>
    !!editable && !!onedit && editable.columns[column] != null && columns[column].kind !== 'binary';

  /** The value to show: a staged edit if there is one, else the fetched value. */
  const valueAt = (row: number, column: number): string | null =>
    edits[row] && column in edits[row] ? edits[row][column] : rows[row][column];
  const isEdited = (row: number, column: number) => !!edits[row] && column in edits[row];

  function toggleSort(column: number) {
    if (sort?.column !== column) sort = { column, direction: 'asc' };
    else if (sort.direction === 'asc') sort = { column, direction: 'desc' };
    else sort = null;
  }

  function startResize(event: PointerEvent, column: number) {
    event.preventDefault();
    event.stopPropagation();
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const startX = event.clientX;
    const startWidth = widths[column];
    const move = (e: PointerEvent) =>
      (resized[column] = Math.max(40, Math.round(startWidth + e.clientX - startX)));
    const stop = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', stop);
      handle.removeEventListener('pointercancel', stop);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', stop);
    handle.addEventListener('pointercancel', stop);
  }

  function reveal(position: number) {
    if (!viewport) return;
    const top = position * ROW;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (top + ROW > viewport.scrollTop + height - HEADER)
      viewport.scrollTop = top + ROW - height + HEADER;
  }

  function beginEdit(row: number, column: number) {
    if (!canEdit(column)) return;
    editing = { row, column, text: valueAt(row, column) ?? '' };
  }

  function commitEdit(value: string | null) {
    if (!editing) return;
    onedit?.(editing.row, editing.column, value);
    editing = null;
    viewport?.focus();
  }

  async function copy() {
    if (!selected) return;
    try {
      await navigator.clipboard.writeText(valueAt(selected.row, selected.column) ?? '');
      copied = true;
      setTimeout(() => (copied = false), 900);
    } catch {
      // Clipboard access can be refused; nothing else to do.
    }
  }

  function onKeydown(event: KeyboardEvent) {
    if (editing || !selected) return;
    const position = order.indexOf(selected.row);
    const shownAt = shown.indexOf(selected.column);
    const move = (rowDelta: number, columnDelta: number) => {
      event.preventDefault();
      const nextPosition = Math.min(order.length - 1, Math.max(0, position + rowDelta));
      const nextColumn = Math.min(shown.length - 1, Math.max(0, shownAt + columnDelta));
      if (order.length === 0) return;
      selected = { row: order[nextPosition], column: shown[nextColumn] };
      reveal(nextPosition);
    };
    const page = Math.max(1, Math.floor((height - HEADER) / ROW) - 1);
    if (event.key === 'ArrowDown') move(1, 0);
    else if (event.key === 'ArrowUp') move(-1, 0);
    else if (event.key === 'ArrowRight') move(0, 1);
    else if (event.key === 'ArrowLeft') move(0, -1);
    else if (event.key === 'PageDown') move(page, 0);
    else if (event.key === 'PageUp') move(-page, 0);
    else if (event.key === 'Home') move(-order.length, 0);
    else if (event.key === 'End') move(order.length, 0);
    else if (event.key === 'Enter' || event.key === 'F2') {
      event.preventDefault();
      beginEdit(selected.row, selected.column);
    } else if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'c') {
      // Leave the browser's own copy alone when text is selected.
      if (window.getSelection()?.toString()) return;
      event.preventDefault();
      void copy();
    }
  }

  function focusInput(node: HTMLInputElement) {
    node.focus();
    node.select();
  }
</script>

<div
  class="grid"
  role="grid"
  aria-rowcount={order.length + 1}
  aria-colcount={shown.length}
  tabindex="0"
  bind:this={viewport}
  onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
  onkeydown={onKeydown}
  style:--grid-font-size="{fontSize}px"
>
  <div
    class="header"
    role="row"
    style:grid-template-columns={template}
    style:width="{totalWidth}px"
  >
    <div class="corner" role="columnheader" aria-label="Row number"></div>
    {#each shown as c (c)}
      {@const column = columns[c]}
      <div
        class="head"
        class:number={column.kind === 'number'}
        role="columnheader"
        aria-sort={sort?.column === c
          ? sort.direction === 'asc'
            ? 'ascending'
            : 'descending'
          : 'none'}
      >
        <button
          class="sort"
          title="{column.name} · {column.typeName}. Click to sort."
          onclick={() => toggleSort(c)}
        >
          <span class="name">
            {column.name}{#if sort?.column === c}<span class="arrow"
                >{sort.direction === 'asc' ? ' ▲' : ' ▼'}</span
              >{/if}
          </span>
          <span class="type">{column.typeName}</span>
        </button>
        <span class="resizer" role="presentation" onpointerdown={(e) => startResize(e, c)}></span>
      </div>
    {/each}
  </div>

  <div class="body" style:height="{order.length * ROW}px" style:width="{totalWidth}px">
    {#each visible as r, i (r)}
      <div
        class="row"
        class:odd={(first + i) % 2 === 1}
        role="row"
        aria-rowindex={first + i + 2}
        style:grid-template-columns={template}
        style:transform="translateY({(first + i) * ROW}px)"
      >
        <div class="gutter" role="rowheader">{r + 1}</div>
        {#each shown as c (c)}
          {@const value = valueAt(r, c)}
          {#if editing?.row === r && editing.column === c}
            <div class="cell editing" role="gridcell">
              <input
                use:focusInput
                bind:value={editing.text}
                aria-label="Edit {columns[c].name}"
                spellcheck="false"
                onkeydown={(e) => {
                  e.stopPropagation();
                  if (e.key === 'Enter') commitEdit(editing!.text);
                  else if (e.key === 'Escape') {
                    editing = null;
                    viewport?.focus();
                  }
                }}
                onblur={() => editing && commitEdit(editing.text)}
              />
              <button
                class="null"
                title="Set to NULL"
                onpointerdown={(e) => e.preventDefault()}
                onclick={() => commitEdit(null)}
              >
                NULL
              </button>
            </div>
          {:else}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div
              class="cell"
              class:null={value === null}
              class:number={columns[c].kind === 'number'}
              class:selected={selected?.row === r && selected.column === c}
              class:edited={isEdited(r, c)}
              role="gridcell"
              tabindex="-1"
              title={value !== null && value.length > 40 ? value.slice(0, 2000) : undefined}
              onclick={() => (selected = { row: r, column: c })}
              ondblclick={() => beginEdit(r, c)}
            >
              {value === null ? nullText : results ? formatCell(value, columns[c], results) : value}
            </div>
          {/if}
        {/each}
      </div>
    {/each}
  </div>
  {#if copied}<div class="toast" role="status">Copied</div>{/if}
</div>

<style>
  .grid {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: auto;
    font-family: var(--grid-font);
    font-size: var(--grid-font-size);
    outline: none;
    contain: strict;
  }
  .header {
    position: sticky;
    top: 0;
    z-index: 3;
    display: grid;
    height: 40px;
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--border-strong);
  }
  .head,
  .corner {
    position: relative;
    display: flex;
    border-right: 1px solid var(--border);
    min-width: 0;
  }
  .corner {
    position: sticky;
    left: 0;
    z-index: 4;
    background: var(--bg-subtle);
  }
  .sort {
    display: flex;
    flex-direction: column;
    justify-content: center;
    flex: 1;
    min-width: 0;
    padding: 0 10px;
    border: none;
    background: none;
    text-align: left;
    font: inherit;
  }
  .sort:hover {
    background: var(--bg-hover);
  }
  .head.number .sort {
    align-items: flex-end;
  }
  .name {
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .arrow {
    color: var(--accent);
    font-size: 0.8em;
  }
  .type {
    font-size: 0.85em;
    color: var(--text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .resizer {
    position: absolute;
    top: 0;
    right: -3px;
    width: 7px;
    height: 100%;
    cursor: col-resize;
    z-index: 1;
  }
  .resizer:hover {
    background: var(--focus);
    opacity: 0.5;
  }
  .body {
    position: relative;
  }
  .row {
    position: absolute;
    top: 0;
    left: 0;
    display: grid;
    height: 24px;
    line-height: 23px;
    border-bottom: 1px solid var(--border);
  }
  .row.odd {
    background: var(--bg-subtle);
  }
  .gutter {
    position: sticky;
    left: 0;
    z-index: 2;
    padding: 0 8px;
    text-align: right;
    color: var(--text-faint);
    background: var(--bg-subtle);
    border-right: 1px solid var(--border);
    font-variant-numeric: tabular-nums;
  }
  .cell {
    padding: 0 10px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: pre;
    border-right: 1px solid var(--border);
    user-select: text;
    -webkit-user-select: text;
  }
  .cell.number {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .cell.null {
    color: var(--text-faint);
    font-style: italic;
  }
  .cell.edited {
    background: rgba(232, 163, 61, 0.28);
  }
  .cell.selected {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
  }
  .cell.editing {
    display: flex;
    padding: 0;
    outline: 2px solid var(--focus);
    outline-offset: -2px;
    background: var(--bg);
  }
  .cell.editing input {
    flex: 1;
    min-width: 0;
    padding: 0 8px;
    border: none;
    outline: none;
    background: none;
    font: inherit;
    color: inherit;
  }
  .cell.editing .null {
    padding: 0 6px;
    border: none;
    border-left: 1px solid var(--border);
    background: var(--bg-subtle);
    font: 10px var(--ui-font);
    color: var(--text-muted);
  }
  .toast {
    position: sticky;
    left: 50%;
    bottom: 12px;
    width: max-content;
    transform: translateX(-50%);
    padding: 4px 12px;
    border-radius: 12px;
    background: var(--text);
    color: var(--bg);
    font: 12px var(--ui-font);
    z-index: 5;
  }
</style>
