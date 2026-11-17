<script lang="ts">
  // Results grid. Only the rows in view are in the DOM, so scrolling stays
  // smooth however many rows are loaded.
  import { settings } from '../lib/settings.svelte';
  import type { Column } from '../lib/types';

  let { columns, rows }: { columns: Column[]; rows: (string | null)[][] } = $props();

  const ROW = 24;
  const HEADER = 40;
  const OVERSCAN = 8;
  const CHAR = 7.4; // average glyph width of the grid font at 12px

  let viewport = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let height = $state(300);
  let selected = $state<{ row: number; column: number } | null>(null);
  let copied = $state(false);

  const nullText = $derived(settings.value?.results.nullText ?? 'NULL');
  const fontSize = $derived(settings.value?.appearance.gridFontSize ?? 12);

  // Column widths from the header and a sample of the data.
  const widths = $derived.by(() => {
    const scale = fontSize / 12;
    return columns.map((column, c) => {
      let longest = Math.max(column.name.length, column.typeName.length * 0.85);
      for (let r = 0; r < Math.min(rows.length, 200); r++) {
        longest = Math.max(longest, rows[r][c]?.length ?? nullText.length);
      }
      return Math.round(Math.min(420, Math.max(64, longest * CHAR * scale + 22)));
    });
  });
  const gutter = $derived(Math.max(44, String(rows.length).length * 8 + 20));
  const template = $derived(`${gutter}px ${widths.map((w) => `${w}px`).join(' ')}`);
  const totalWidth = $derived(gutter + widths.reduce((a, b) => a + b, 0));

  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN));
  const last = $derived(Math.min(rows.length, Math.ceil((scrollTop + height) / ROW) + OVERSCAN));
  const visible = $derived(rows.slice(first, last));

  // A new result starts at the top with nothing selected.
  $effect(() => {
    void rows;
    selected = null;
    if (viewport) viewport.scrollTop = 0;
  });

  $effect(() => {
    if (!viewport) return;
    const observer = new ResizeObserver(() => (height = viewport!.clientHeight));
    observer.observe(viewport);
    return () => observer.disconnect();
  });

  function reveal(row: number) {
    if (!viewport) return;
    const top = row * ROW;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (top + ROW > viewport.scrollTop + height - HEADER) {
      viewport.scrollTop = top + ROW - height + HEADER;
    }
  }

  async function copy() {
    if (!selected) return;
    const value = rows[selected.row]?.[selected.column];
    try {
      await navigator.clipboard.writeText(value ?? '');
      copied = true;
      setTimeout(() => (copied = false), 900);
    } catch {
      // Clipboard access can be refused; nothing else to do.
    }
  }

  function onKeydown(event: KeyboardEvent) {
    if (!selected) return;
    const move = (rowDelta: number, columnDelta: number) => {
      event.preventDefault();
      selected = {
        row: Math.min(rows.length - 1, Math.max(0, selected!.row + rowDelta)),
        column: Math.min(columns.length - 1, Math.max(0, selected!.column + columnDelta)),
      };
      reveal(selected.row);
    };
    const page = Math.max(1, Math.floor((height - HEADER) / ROW) - 1);
    if (event.key === 'ArrowDown') move(1, 0);
    else if (event.key === 'ArrowUp') move(-1, 0);
    else if (event.key === 'ArrowRight') move(0, 1);
    else if (event.key === 'ArrowLeft') move(0, -1);
    else if (event.key === 'PageDown') move(page, 0);
    else if (event.key === 'PageUp') move(-page, 0);
    else if (event.key === 'Home') move(-rows.length, 0);
    else if (event.key === 'End') move(rows.length, 0);
    else if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'c') {
      event.preventDefault();
      void copy();
    }
  }
</script>

<div
  class="grid"
  role="grid"
  aria-rowcount={rows.length + 1}
  aria-colcount={columns.length}
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
    {#each columns as column, c (c)}
      <div
        class="head"
        class:number={column.kind === 'number'}
        role="columnheader"
        title="{column.name} · {column.typeName}"
      >
        <span class="name">{column.name}</span>
        <span class="type">{column.typeName}</span>
      </div>
    {/each}
  </div>

  <div class="body" style:height="{rows.length * ROW}px" style:width="{totalWidth}px">
    {#each visible as row, i (first + i)}
      {@const r = first + i}
      <div
        class="row"
        class:odd={r % 2 === 1}
        role="row"
        aria-rowindex={r + 2}
        style:grid-template-columns={template}
        style:transform="translateY({r * ROW}px)"
      >
        <div class="gutter" role="rowheader">{r + 1}</div>
        {#each row as value, c (c)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="cell"
            class:null={value === null}
            class:number={columns[c]?.kind === 'number'}
            class:selected={selected?.row === r && selected.column === c}
            role="gridcell"
            tabindex="-1"
            title={value !== null && value.length > 40 ? value.slice(0, 2000) : undefined}
            onclick={() => (selected = { row: r, column: c })}
          >
            {value === null ? nullText : value.length > 300 ? `${value.slice(0, 300)}…` : value}
          </div>
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
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: 0 10px;
    border-right: 1px solid var(--border);
    min-width: 0;
  }
  .corner {
    position: sticky;
    left: 0;
    z-index: 4;
    background: var(--bg-subtle);
  }
  .head.number {
    align-items: flex-end;
  }
  .name {
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .type {
    font-size: 0.85em;
    color: var(--text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
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
  .cell.selected {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
    background: var(--bg-active);
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
