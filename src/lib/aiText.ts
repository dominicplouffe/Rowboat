// Text helpers for the copilot. Pure, so they can be tested without an editor.

/**
 * The `--` comment at or directly above the cursor, as a prompt: writing
 * `-- orders per customer last month` and pressing the Ask shortcut is enough.
 * Returns the prompt and the comment's range, or null when there is none.
 */
export function commentPrompt(
  text: string,
  cursor: number,
): { prompt: string; from: number; to: number } | null {
  const lines = text.split('\n');
  const starts: number[] = [];
  let offset = 0;
  for (const line of lines) {
    starts.push(offset);
    offset += line.length + 1;
  }
  let row = starts.length - 1;
  while (row > 0 && starts[row] > cursor) row--;
  const isComment = (i: number) => /^\s*--/.test(lines[i] ?? '');
  // On an empty line just under a comment, the comment still counts.
  if (!isComment(row) && lines[row].trim() === '' && row > 0 && isComment(row - 1)) row--;
  if (!isComment(row)) return null;
  let first = row;
  while (first > 0 && isComment(first - 1)) first--;
  let last = row;
  while (last < lines.length - 1 && isComment(last + 1)) last++;
  const prompt = lines
    .slice(first, last + 1)
    .map((line) => line.replace(/^\s*--+\s?/, '').trim())
    .filter(Boolean)
    .join(' ');
  if (!prompt) return null;
  return { prompt, from: starts[first], to: starts[last] + lines[last].length };
}

/** `1234` → `1,234`; null (the provider did not say) → `?`. */
export function tokenCount(n: number | null): string {
  return n === null ? '?' : n.toLocaleString('en-US');
}

/** The streamed answer without its code fence, for showing while it arrives. */
export function streamingSql(text: string): { sql: string; rest: string } {
  const open = text.indexOf('```');
  if (open < 0) return { sql: '', rest: text.trim() };
  const afterFence = text.slice(open + 3);
  const newline = afterFence.indexOf('\n');
  if (newline < 0) return { sql: '', rest: text.slice(0, open).trim() };
  const body = afterFence.slice(newline + 1);
  const close = body.indexOf('```');
  if (close < 0) return { sql: body, rest: text.slice(0, open).trim() };
  return {
    sql: body.slice(0, close).trimEnd(),
    rest: `${text.slice(0, open).trim()} ${body.slice(close + 3).trim()}`.trim(),
  };
}
