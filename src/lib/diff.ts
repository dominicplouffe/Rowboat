// A small line diff for showing an AI fix against the statement that failed.

export interface DiffLine {
  kind: 'same' | 'removed' | 'added';
  text: string;
}

/** Longest-common-subsequence diff, line by line. Statements are short. */
export function diffLines(before: string, after: string): DiffLine[] {
  const a = before.replace(/\r\n/g, '\n').split('\n');
  const b = after.replace(/\r\n/g, '\n').split('\n');
  // lcs[i][j] = length of the LCS of a[i..] and b[j..]
  const lcs: number[][] = Array.from({ length: a.length + 1 }, () =>
    new Array<number>(b.length + 1).fill(0),
  );
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      out.push({ kind: 'same', text: a[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      out.push({ kind: 'removed', text: a[i++] });
    } else {
      out.push({ kind: 'added', text: b[j++] });
    }
  }
  while (i < a.length) out.push({ kind: 'removed', text: a[i++] });
  while (j < b.length) out.push({ kind: 'added', text: b[j++] });
  return out;
}

export interface Segment {
  text: string;
  changed: boolean;
}

/**
 * For a removed line and the added line that replaces it: the parts that
 * differ, so a one-word fix does not light up the whole line.
 */
export function changedSpan(
  before: string,
  after: string,
): { before: Segment[]; after: Segment[] } {
  let start = 0;
  const max = Math.min(before.length, after.length);
  while (start < max && before[start] === after[start]) start++;
  let end = 0;
  while (end < max - start && before[before.length - 1 - end] === after[after.length - 1 - end]) {
    end++;
  }
  const cut = (text: string): Segment[] =>
    [
      { text: text.slice(0, start), changed: false },
      { text: text.slice(start, text.length - end), changed: true },
      { text: text.slice(text.length - end), changed: false },
    ].filter((s) => s.text.length > 0);
  return { before: cut(before), after: cut(after) };
}
