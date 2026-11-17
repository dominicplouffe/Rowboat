// How result values are shown and ordered in the grid. Values arrive from the
// core as text (or null); these functions never change the data, only its
// display, following Settings → Results.

import type { Column, Settings } from './types';

type ResultSettings = Settings['results'];

const TIMESTAMP = /^(\d{4})-(\d\d)-(\d\d)[ T](\d\d):(\d\d):(\d\d)(\.\d+)?(Z|[+-]\d\d(?::?\d\d)?)?$/;
const PLAIN_NUMBER = /^-?\d+(\.\d+)?$/;

/** True for types whose values are instants in UTC rather than wall-clock times. */
function isInstant(column: Column, zone: string | undefined): boolean {
  const type = column.typeName.toLowerCase();
  return zone !== undefined || type === 'timestamptz' || type.startsWith('datetime');
}

const pad = (n: number, width = 2) => String(n).padStart(width, '0');

function formatTimestamp(value: string, column: Column, settings: ResultSettings): string {
  const match = TIMESTAMP.exec(value);
  if (!match) return value;
  const [, y, mo, d, h, mi, s, fraction = '', zone] = match;
  let parts = { y: +y, mo: +mo, d: +d, h: +h, mi: +mi, s: +s };
  let offsetMinutes = 0;

  if (isInstant(column, zone)) {
    // Normalize to UTC, then to the chosen zone.
    const zoneMatch = zone && zone !== 'Z' ? /^([+-])(\d\d)(?::?(\d\d))?$/.exec(zone) : null;
    const zoneOffset = zoneMatch
      ? (zoneMatch[1] === '-' ? -1 : 1) * (+zoneMatch[2] * 60 + +(zoneMatch[3] ?? 0))
      : 0;
    const utc =
      Date.UTC(parts.y, parts.mo - 1, parts.d, parts.h, parts.mi, parts.s) - zoneOffset * 60_000;
    if (settings.timeZone === 'local') {
      const local = new Date(utc);
      offsetMinutes = -local.getTimezoneOffset();
      parts = {
        y: local.getFullYear(),
        mo: local.getMonth() + 1,
        d: local.getDate(),
        h: local.getHours(),
        mi: local.getMinutes(),
        s: local.getSeconds(),
      };
    } else {
      const date = new Date(utc);
      parts = {
        y: date.getUTCFullYear(),
        mo: date.getUTCMonth() + 1,
        d: date.getUTCDate(),
        h: date.getUTCHours(),
        mi: date.getUTCMinutes(),
        s: date.getUTCSeconds(),
      };
    }
  }

  const sign = offsetMinutes < 0 ? '-' : '+';
  const abs = Math.abs(offsetMinutes);
  const tokens: Record<string, string> = {
    YYYY: pad(parts.y, 4),
    MM: pad(parts.mo),
    DD: pad(parts.d),
    HH: pad(parts.h),
    mm: pad(parts.mi),
    // Fractional seconds are data: keep them wherever seconds are shown.
    ss: pad(parts.s) + fraction,
    Z: `${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`,
  };
  return settings.dateTimeFormat.replace(/YYYY|MM|DD|HH|mm|ss|Z/g, (token) => tokens[token]);
}

function groupDigits(value: string): string {
  if (!PLAIN_NUMBER.test(value)) return value;
  const [whole, fraction] = value.split('.');
  const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return fraction === undefined ? grouped : `${grouped}.${fraction}`;
}

function hexToBase64(value: string): string {
  const hex = value.slice(2);
  let binary = '';
  for (let i = 0; i + 1 < hex.length; i += 2)
    binary += String.fromCharCode(parseInt(hex.slice(i, i + 2), 16));
  return btoa(binary);
}

/** The text to show for a non-null value. */
export function formatCell(value: string, column: Column, settings: ResultSettings): string {
  let text = value;
  if (column.kind === 'number' && settings.numberFormat === 'grouped') text = groupDigits(value);
  else if (column.kind === 'temporal') text = formatTimestamp(value, column, settings);
  else if (
    column.kind === 'binary' &&
    settings.binaryDisplay === 'base64' &&
    value.startsWith('\\x')
  ) {
    text = hexToBase64(value);
  }
  return text.length > settings.maxCellChars ? `${text.slice(0, settings.maxCellChars)}…` : text;
}

/** Whether a cell's value is JSON worth opening in the viewer. */
export function looksLikeJson(value: string | null, column: Column, autoDetect: boolean): boolean {
  if (value === null) return false;
  if (column.kind === 'json') return true;
  if (!autoDetect || column.kind === 'number') return false;
  const first = value.trimStart()[0];
  if (first !== '{' && first !== '[') return false;
  try {
    JSON.parse(value);
    return true;
  } catch {
    return false;
  }
}

export type SortDirection = 'asc' | 'desc';

/** Orders two values of a column. NULLs sort last in either direction. */
export function compareCells(
  a: string | null,
  b: string | null,
  column: Column,
  direction: SortDirection,
): number {
  if (a === null || b === null) return a === b ? 0 : a === null ? 1 : -1;
  let order: number;
  if (column.kind === 'number') {
    const x = Number(a);
    const y = Number(b);
    order = Number.isNaN(x) || Number.isNaN(y) ? (a < b ? -1 : a > b ? 1 : 0) : x - y;
  } else {
    order = a < b ? -1 : a > b ? 1 : 0;
  }
  return direction === 'asc' ? order : -order;
}

export interface GridView {
  sort: { column: number; direction: SortDirection } | null;
  /** Rows must contain this text in some column (case-insensitive). */
  filter: string;
}

/**
 * The row indexes to show, in order: filtered, then sorted. Works on indexes
 * so the rows themselves are never copied or reordered.
 */
export function viewRows(rows: (string | null)[][], columns: Column[], view: GridView): number[] {
  const needle = view.filter.trim().toLowerCase();
  let order: number[] = [];
  for (let i = 0; i < rows.length; i++) {
    if (!needle || rows[i].some((cell) => cell !== null && cell.toLowerCase().includes(needle)))
      order.push(i);
  }
  if (view.sort) {
    const { column, direction } = view.sort;
    const meta = columns[column];
    // Stable: equal values keep their fetched order.
    order = order
      .map((row, position) => ({ row, position }))
      .sort(
        (a, b) =>
          compareCells(rows[a.row][column], rows[b.row][column], meta, direction) ||
          a.position - b.position,
      )
      .map((entry) => entry.row);
  }
  return order;
}
