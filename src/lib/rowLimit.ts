// Row limits are applied at fetch time by the Rust core (server-side cursor on
// PostgreSQL, max_result_rows on ClickHouse, stop stepping on SQLite). The UI
// only chooses the number; it never rewrites SQL. 0 means "no limit".

export const ROW_LIMIT_PRESETS = [100, 500, 1000, 5000] as const;
export const NO_LIMIT = 0;
export const MAX_ROW_LIMIT = 1_000_000;

export function formatRowLimit(limit: number): string {
  return limit === NO_LIMIT ? 'No limit' : `${limit.toLocaleString('en-US')} rows`;
}

/** Parses a custom limit typed by the user; null when it is not a valid count. */
export function parseRowLimit(text: string): number | null {
  const cleaned = text.replace(/[,_\s]/g, '');
  if (!/^\d+$/.test(cleaned)) return null;
  const value = Number(cleaned);
  if (value < 1 || value > MAX_ROW_LIMIT) return null;
  return value;
}
