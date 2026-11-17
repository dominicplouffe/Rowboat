// Schema-aware SQL completion.
//
// Pure functions over text and a schema snapshot, so the behaviour can be
// tested without an editor and stays fast: everything works from an in-memory
// index built once per snapshot (see `buildIndex`), never from the database.

import type { DbKind, SchemaSnapshot, SnapshotTable } from './types';

export type CompletionKind =
  'table' | 'view' | 'column' | 'function' | 'keyword' | 'schema' | 'join' | 'alias';

export interface Completion {
  label: string;
  kind: CompletionKind;
  /** Secondary text: a column's type and table, a function's arguments. */
  detail: string;
  /** What to insert when it differs from the label (quoting, join clauses). */
  apply?: string;
  /** Key under which accepted completions are counted for ranking. */
  usageKey: string;
}

export interface CompletionResult {
  /** Start of the text the completions replace. */
  from: number;
  options: Completion[];
}

interface IndexedTable {
  table: SnapshotTable;
  lower: string;
  /** True when the table can be written without its schema. */
  onPath: boolean;
}

export interface SchemaIndex {
  kind: DbKind;
  snapshot: SchemaSnapshot;
  tables: IndexedTable[];
  /** Lowercased `schema.table` and bare `table` (for on-path tables) → table. */
  byName: Map<string, SnapshotTable>;
  schemas: string[];
  functions: { name: string; lower: string; signature: string }[];
}

const KEYWORDS = [
  'SELECT',
  'FROM',
  'WHERE',
  'GROUP BY',
  'ORDER BY',
  'HAVING',
  'LIMIT',
  'OFFSET',
  'JOIN',
  'LEFT JOIN',
  'RIGHT JOIN',
  'INNER JOIN',
  'FULL JOIN',
  'CROSS JOIN',
  'ON',
  'AS',
  'AND',
  'OR',
  'NOT',
  'IN',
  'IS NULL',
  'IS NOT NULL',
  'LIKE',
  'BETWEEN',
  'EXISTS',
  'CASE',
  'WHEN',
  'THEN',
  'ELSE',
  'END',
  'DISTINCT',
  'UNION',
  'UNION ALL',
  'INSERT INTO',
  'VALUES',
  'UPDATE',
  'SET',
  'DELETE FROM',
  'CREATE TABLE',
  'ALTER TABLE',
  'DROP TABLE',
  'WITH',
  'ASC',
  'DESC',
  'NULL',
  'TRUE',
  'FALSE',
  'EXPLAIN',
  'RETURNING',
  'USING',
];
const DIALECT_KEYWORDS: Record<DbKind, string[]> = {
  postgres: [
    'ILIKE',
    'EXPLAIN ANALYZE',
    'ON CONFLICT',
    'DO UPDATE',
    'DO NOTHING',
    'LATERAL',
    'MATERIALIZED',
  ],
  clickhouse: [
    'FORMAT',
    'SETTINGS',
    'ARRAY JOIN',
    'PREWHERE',
    'FINAL',
    'SAMPLE',
    'GLOBAL IN',
    'ILIKE',
    'LIMIT BY',
  ],
  sqlite: ['GLOB', 'PRAGMA', 'ON CONFLICT', 'REPLACE INTO', 'VACUUM', 'WITHOUT ROWID'],
};

/** Words that end a table reference, so they are never mistaken for an alias. */
const NOT_ALIAS = new Set([
  'where',
  'join',
  'left',
  'right',
  'inner',
  'full',
  'cross',
  'on',
  'group',
  'order',
  'having',
  'limit',
  'offset',
  'union',
  'using',
  'set',
  'values',
  'returning',
  'select',
  'from',
  'natural',
  'window',
  'as',
  'prewhere',
  'final',
  'sample',
  'array',
  'format',
  'settings',
  'lateral',
  'outer',
  'except',
  'intersect',
]);

export function buildIndex(kind: DbKind, snapshot: SchemaSnapshot): SchemaIndex {
  const path = new Set(snapshot.searchPath.map((s) => s.toLowerCase()));
  const byName = new Map<string, SnapshotTable>();
  const tables = snapshot.tables.map((table) => {
    const onPath = !table.schema || path.has(table.schema.toLowerCase());
    if (table.schema) byName.set(`${table.schema}.${table.name}`.toLowerCase(), table);
    if (onPath && !byName.has(table.name.toLowerCase()))
      byName.set(table.name.toLowerCase(), table);
    return { table, lower: table.name.toLowerCase(), onPath };
  });
  return {
    kind,
    snapshot,
    tables,
    byName,
    schemas: [...new Set(snapshot.tables.map((t) => t.schema).filter(Boolean))],
    functions: snapshot.functions.map((f) => ({ ...f, lower: f.name.toLowerCase() })),
  };
}

// ----------------------------------------------------------------- lexing ----

interface Token {
  /** `word` covers identifiers and keywords; quoted identifiers are words too. */
  type: 'word' | 'dot' | 'comma' | 'open' | 'close' | 'semicolon' | 'other';
  text: string;
  /** For words: the identifier without quotes. */
  value: string;
  start: number;
  end: number;
}

const TOKEN =
  /--[^\n]*|\/\*[\s\S]*?(?:\*\/|$)|'(?:[^']|'')*'?|\$([A-Za-z_]*)\$[\s\S]*?(?:\$\1\$|$)|"(?:[^"]|"")*"?|`(?:[^`\\]|\\.)*`?|[A-Za-z_][\w$]*|\d[\w.]*|[.,();]|\S/g;

function tokenize(text: string): Token[] {
  const tokens: Token[] = [];
  for (const match of text.matchAll(TOKEN)) {
    const t = match[0];
    const start = match.index;
    const end = start + t.length;
    const first = t[0];
    if (
      t.startsWith('--') ||
      t.startsWith('/*') ||
      first === "'" ||
      (first === '$' && t.length > 1)
    )
      continue;
    if (first === '"' || first === '`') {
      const value = t
        .slice(1, t.endsWith(first) && t.length > 1 ? -1 : undefined)
        .replace(/""/g, '"');
      tokens.push({ type: 'word', text: t, value, start, end });
    } else if (/^[A-Za-z_]/.test(t)) tokens.push({ type: 'word', text: t, value: t, start, end });
    else if (t === '.') tokens.push({ type: 'dot', text: t, value: t, start, end });
    else if (t === ',') tokens.push({ type: 'comma', text: t, value: t, start, end });
    else if (t === '(') tokens.push({ type: 'open', text: t, value: t, start, end });
    else if (t === ')') tokens.push({ type: 'close', text: t, value: t, start, end });
    else if (t === ';') tokens.push({ type: 'semicolon', text: t, value: t, start, end });
    else tokens.push({ type: 'other', text: t, value: t, start, end });
  }
  return tokens;
}

const isKeyword = (token: Token | undefined, ...words: string[]) =>
  token?.type === 'word' &&
  token.text[0] !== '"' &&
  token.text[0] !== '`' &&
  words.includes(token.value.toLowerCase());

// ------------------------------------------------------------------ scope ----

/** A table (or CTE or subquery) visible in the statement, under a name. */
export interface ScopeEntry {
  /** The name it is referred to by: the alias, else the table name. */
  name: string;
  hasAlias: boolean;
  table: SnapshotTable | null;
  /** Columns, for real tables and for CTEs/subqueries whose list could be read. */
  columns: { name: string; typeName: string }[];
}

/** Output column names of a SELECT list: aliases, else the last identifier. */
function selectListNames(tokens: Token[], start: number, end: number): string[] {
  const names: string[] = [];
  let depth = 0;
  let last: Token | null = null;
  let afterAs: Token | null = null;
  const flush = () => {
    const chosen = afterAs ?? last;
    if (chosen && chosen.value !== '*') names.push(chosen.value);
    last = afterAs = null;
  };
  for (let i = start; i < end; i++) {
    const token = tokens[i];
    if (token.type === 'open') depth++;
    else if (token.type === 'close') depth--;
    else if (depth === 0) {
      if (isKeyword(token, 'from')) break;
      if (token.type === 'comma') flush();
      else if (token.type === 'word') {
        if (isKeyword(tokens[i - 1], 'as')) afterAs = token;
        else if (!isKeyword(token, 'as', 'distinct', 'all')) last = token;
      }
    }
  }
  flush();
  return names;
}

/** Index of the token closing the parenthesis opened at `open`. */
function closing(tokens: Token[], open: number): number {
  let depth = 0;
  for (let i = open; i < tokens.length; i++) {
    if (tokens[i].type === 'open') depth++;
    else if (tokens[i].type === 'close' && --depth === 0) return i;
  }
  return tokens.length;
}

function selectStart(tokens: Token[], from: number, to: number): number {
  for (let i = from; i < to; i++) if (isKeyword(tokens[i], 'select')) return i + 1;
  return -1;
}

/** Finds every table reference, CTE and subquery alias in a statement. */
export function resolveScope(index: SchemaIndex, statement: string): ScopeEntry[] {
  const tokens = tokenize(statement);
  const scope: ScopeEntry[] = [];
  const ctes = new Map<string, ScopeEntry['columns']>();

  // WITH name [(columns)] AS ( select … )
  for (let i = 0; i < tokens.length; i++) {
    const afterWith =
      isKeyword(tokens[i - 1], 'with', 'recursive') ||
      (tokens[i - 1]?.type === 'comma' && ctes.size > 0);
    if (tokens[i].type !== 'word' || !afterWith) continue;
    let j = i + 1;
    let explicit: string[] | null = null;
    if (tokens[j]?.type === 'open' && !isKeyword(tokens[j - 1], 'as')) {
      const end = closing(tokens, j);
      explicit = tokens
        .slice(j + 1, end)
        .filter((t) => t.type === 'word')
        .map((t) => t.value);
      j = end + 1;
    }
    if (!isKeyword(tokens[j], 'as')) continue;
    while (isKeyword(tokens[j + 1], 'not', 'materialized')) j++;
    if (tokens[j + 1]?.type !== 'open') continue;
    const end = closing(tokens, j + 1);
    const start = selectStart(tokens, j + 2, end);
    const names = explicit ?? (start >= 0 ? selectListNames(tokens, start, end) : []);
    ctes.set(
      tokens[i].value.toLowerCase(),
      names.map((name) => ({ name, typeName: '' })),
    );
    i = end;
  }

  const addReference = (i: number): number => {
    // Subquery: ( select … ) alias
    if (tokens[i]?.type === 'open') {
      const end = closing(tokens, i);
      const start = selectStart(tokens, i + 1, end);
      let j = end + 1;
      if (isKeyword(tokens[j], 'as')) j++;
      const alias = tokens[j];
      if (start >= 0 && alias?.type === 'word' && !NOT_ALIAS.has(alias.value.toLowerCase())) {
        scope.push({
          name: alias.value,
          hasAlias: true,
          table: null,
          columns: selectListNames(tokens, start, end).map((name) => ({ name, typeName: '' })),
        });
        return j + 1;
      }
      return end + 1;
    }
    if (tokens[i]?.type !== 'word' || NOT_ALIAS.has(tokens[i].value.toLowerCase())) return i;
    const parts = [tokens[i].value];
    let j = i + 1;
    while (tokens[j]?.type === 'dot' && tokens[j + 1]?.type === 'word') {
      parts.push(tokens[j + 1].value);
      j += 2;
    }
    if (tokens[j]?.type === 'open') return j; // a table function, e.g. generate_series(…)
    if (isKeyword(tokens[j], 'as')) j++;
    let alias: string | null = null;
    if (tokens[j]?.type === 'word' && !NOT_ALIAS.has(tokens[j].value.toLowerCase())) {
      alias = tokens[j].value;
      j++;
    }
    const key = parts.join('.').toLowerCase();
    const cte = parts.length === 1 ? ctes.get(key) : undefined;
    const table = cte ? null : (index.byName.get(key) ?? null);
    scope.push({
      name: alias ?? parts[parts.length - 1],
      hasAlias: alias !== null,
      table,
      columns: cte ?? table?.columns.map((c) => ({ name: c.name, typeName: c.typeName })) ?? [],
    });
    return j;
  };

  for (let i = 0; i < tokens.length; i++) {
    if (!isKeyword(tokens[i], 'from', 'join', 'update', 'into')) continue;
    // "DELETE FROM", "INSERT INTO" and friends all introduce a table.
    let j = addReference(i + 1);
    // FROM a, b, c
    while (isKeyword(tokens[i], 'from') && tokens[j]?.type === 'comma') j = addReference(j + 1);
  }
  return scope;
}

// ---------------------------------------------------------------- matching ----

/**
 * Fuzzy match: every character of `query` must appear in `candidate` in
 * order. Higher is better; null means no match. Prefixes and matches at word
 * starts (after `_`) score highest, so `ordcust` finds `order_customers`.
 */
export function fuzzyScore(query: string, candidate: string): number | null {
  if (!query) return 0;
  if (candidate.startsWith(query)) return 1000 - candidate.length;
  let score = 0;
  let at = 0;
  let previous = -2;
  for (const ch of query) {
    const found = candidate.indexOf(ch, at);
    if (found === -1) return null;
    if (found === previous + 1) score += 8; // contiguous
    if (found === 0 || candidate[found - 1] === '_' || candidate[found - 1] === '.') score += 12; // word start
    score -= found - at; // gaps cost
    previous = found;
    at = found + 1;
  }
  return score - candidate.length * 0.1;
}

const BARE = /^[a-z_][a-z0-9_]*$/;
const RESERVED = new Set([
  'select',
  'from',
  'where',
  'order',
  'group',
  'user',
  'table',
  'column',
  'limit',
  'default',
  'check',
  'to',
  'in',
  'on',
  'as',
  'and',
  'or',
  'not',
  'null',
  'end',
  'case',
  'when',
  'then',
  'else',
  'desc',
  'asc',
  'all',
  'any',
]);

/** Quotes an identifier only when the database needs it. */
export function quote(kind: DbKind, name: string): string {
  if (BARE.test(name) && !RESERVED.has(name)) return name;
  // ClickHouse and SQLite keep unquoted case; only PostgreSQL folds it.
  if (
    kind !== 'postgres' &&
    /^[A-Za-z_][A-Za-z0-9_]*$/.test(name) &&
    !RESERVED.has(name.toLowerCase())
  )
    return name;
  return kind === 'clickhouse'
    ? `\`${name.replace(/`/g, '\\`')}\``
    : `"${name.replace(/"/g, '""')}"`;
}

/** A short alias from a table name's word initials: `order_items` → `oi`. */
export function aliasFor(table: string, taken: Set<string>): string {
  const initials = table
    .split(/[_\W]+|(?<=[a-z])(?=[A-Z])/)
    .filter(Boolean)
    .map((word) => word[0].toLowerCase())
    .join('');
  const base = initials || table.slice(0, 1).toLowerCase();
  let alias = base;
  for (let n = 2; taken.has(alias) || RESERVED.has(alias); n++) alias = `${base}${n}`;
  return alias;
}

// ------------------------------------------------------------- completion ----

type Context = 'table' | 'join' | 'expression' | 'any';

const TABLE_WORDS = ['from', 'into', 'update', 'table', 'truncate', 'describe', 'desc'];
const EXPRESSION_WORDS = [
  'select',
  'where',
  'on',
  'and',
  'or',
  'by',
  'having',
  'set',
  'returning',
  'when',
  'then',
  'else',
  'not',
  'in',
  'between',
  'like',
  'distinct',
  'case',
  'using',
  'prewhere',
  'values',
  'as',
];

/** What kind of thing belongs at the cursor, from the clause keyword before it. */
function contextAt(tokens: Token[]): Context {
  let depth = 0;
  for (let i = tokens.length - 1; i >= 0; i--) {
    const token = tokens[i];
    if (token.type === 'close') depth++;
    else if (token.type === 'open') {
      if (depth === 0) {
        // Inside "(": a function call or subquery — expressions, unless the
        // text since the parenthesis has its own clause keyword.
        return 'expression';
      }
      depth--;
    } else if (depth === 0 && token.type === 'word') {
      const word = token.value.toLowerCase();
      if (word === 'join') return 'join';
      if (TABLE_WORDS.includes(word)) return 'table';
      if (EXPRESSION_WORDS.includes(word)) return 'expression';
    }
  }
  return 'any';
}

/** The statement around `cursor`, and the cursor's offset within it. */
function statementAround(text: string, cursor: number): { statement: string; offset: number } {
  let start = 0;
  let end = text.length;
  for (const token of tokenize(text)) {
    if (token.type !== 'semicolon') continue;
    if (token.end <= cursor) start = token.end;
    else {
      end = token.start;
      break;
    }
  }
  return { statement: text.slice(start, end), offset: start };
}

export interface CompleteOptions {
  /** How often each completion has been accepted, by `usageKey`. */
  usage?: Record<string, number>;
  limit?: number;
}

/** Completions at `cursor`, ranked by scope, then past use, then match quality. */
export function complete(
  index: SchemaIndex,
  text: string,
  cursor: number,
  options: CompleteOptions = {},
): CompletionResult | null {
  const { kind } = index;
  const { statement, offset } = statementAround(text, cursor);
  const local = cursor - offset;
  const before = statement.slice(0, local);

  // The word being typed, and anything it is qualified by ("alias." / "schema.table.").
  const word = /(?:[A-Za-z_][\w$]*|"[^"]*"?|`[^`]*`?)?$/.exec(before)![0];
  const from = cursor - word.length;
  const head = before.slice(0, before.length - word.length);
  const qualifier = /((?:(?:[A-Za-z_][\w$]*|"[^"]*"|`[^`]*`)\s*\.\s*)+)$/.exec(head)?.[1];
  const query = word.replace(/^["`]|["`]$/g, '').toLowerCase();
  const usage = options.usage ?? {};

  const scored: { completion: Completion; rank: number }[] = [];
  const add = (completion: Completion, scopeBoost: number) => {
    const score = fuzzyScore(query, completion.label.toLowerCase());
    if (score === null) return;
    const used = Math.min(usage[completion.usageKey] ?? 0, 20);
    scored.push({ completion, rank: scopeBoost * 10_000 + used * 400 + score });
  };

  const tableCompletion = (entry: IndexedTable, qualified: boolean): Completion => {
    const { table } = entry;
    const name = quote(kind, table.name);
    return {
      label: qualified || entry.onPath ? table.name : `${table.schema}.${table.name}`,
      kind: table.kind === 'view' ? 'view' : 'table',
      detail: table.schema ? `${table.kind} in ${table.schema}` : table.kind,
      apply: qualified || entry.onPath ? name : `${quote(kind, table.schema)}.${name}`,
      usageKey: `t:${table.schema}.${table.name}`,
    };
  };
  const columnCompletion = (
    column: { name: string; typeName: string },
    owner: string,
  ): Completion => ({
    label: column.name,
    kind: 'column',
    detail: [column.typeName, owner].filter(Boolean).join(' · '),
    apply: quote(kind, column.name),
    usageKey: `c:${owner}.${column.name}`,
  });

  const scope = resolveScope(index, statement);

  if (qualifier) {
    // After "x.": columns of alias/table x, or objects of schema x.
    const parts = tokenize(qualifier)
      .filter((t) => t.type === 'word')
      .map((t) => t.value.toLowerCase());
    const name = parts.join('.');
    const entry = scope.find((s) => s.name.toLowerCase() === name);
    if (entry && (entry.table || entry.columns.length > 0)) {
      for (const column of entry.columns)
        add(columnCompletion(column, entry.table?.name ?? entry.name), 3);
    } else {
      const table = index.byName.get(name);
      if (table) for (const column of table.columns) add(columnCompletion(column, table.name), 3);
      for (const t of index.tables) {
        if (t.table.schema.toLowerCase() === name) add(tableCompletion(t, true), 2);
      }
    }
    if (scored.length === 0) return null;
  } else {
    const tokens = tokenize(before.slice(0, before.length - word.length));
    const context = contextAt(tokens);

    if (context === 'table' || context === 'join' || context === 'any') {
      for (const t of index.tables) add(tableCompletion(t, false), t.onPath ? 2 : 1);
      for (const schema of index.schemas) {
        add(
          {
            label: schema,
            kind: 'schema',
            detail: 'schema',
            apply: quote(kind, schema),
            usageKey: `s:${schema}`,
          },
          1,
        );
      }
    }

    if (context === 'join') {
      // Whole join clauses from foreign keys touching a table already in scope.
      const taken = new Set(scope.map((s) => s.name.toLowerCase()));
      for (const entry of scope) {
        if (!entry.table) continue;
        const here = entry.table;
        for (const candidate of index.snapshot.tables) {
          const pairs: [string[], string[]][] = [];
          // candidate → here, and here → candidate.
          for (const fk of candidate.foreignKeys) {
            if (fk.refTable === here.name && (fk.refSchema || '') === (here.schema || ''))
              pairs.push([fk.columns, fk.refColumns]);
          }
          for (const fk of here.foreignKeys) {
            if (fk.refTable === candidate.name && (fk.refSchema || '') === (candidate.schema || ''))
              pairs.push([fk.refColumns, fk.columns]);
          }
          for (const [theirs, ours] of pairs) {
            const alias = aliasFor(candidate.name, taken);
            const onPath =
              !candidate.schema || index.snapshot.searchPath.includes(candidate.schema);
            const target = onPath
              ? quote(kind, candidate.name)
              : `${quote(kind, candidate.schema)}.${quote(kind, candidate.name)}`;
            const conditions = theirs.map(
              (column, i) =>
                `${alias}.${quote(kind, column)} = ${quote(kind, entry.name)}.${quote(kind, ours[i])}`,
            );
            const clause = `${target} ${alias} ON ${conditions.join(' AND ')}`;
            add(
              {
                label: clause,
                kind: 'join',
                detail: 'foreign key',
                usageKey: `j:${candidate.name}:${theirs.join(',')}`,
              },
              4,
            );
          }
        }
      }
    }

    if (context === 'expression' || context === 'any') {
      for (const entry of scope) {
        for (const column of entry.columns)
          add(columnCompletion(column, entry.table?.name ?? entry.name), 3);
        if (entry.hasAlias || scope.length > 1) {
          add(
            {
              label: entry.name,
              kind: 'alias',
              detail: entry.table ? `alias of ${entry.table.name}` : 'query alias',
              apply: quote(kind, entry.name),
              usageKey: `a:${entry.name}`,
            },
            3,
          );
        }
      }
      // Functions and keywords only once something is typed: an empty prefix
      // would bury the columns under thousands of names.
      if (query) {
        for (const fn of index.functions) {
          if (fuzzyScore(query, fn.lower) === null) continue;
          add(
            {
              label: fn.name,
              kind: 'function',
              detail: fn.signature,
              apply: `${fn.name}(`,
              usageKey: `f:${fn.name}`,
            },
            1,
          );
        }
      }
    }

    if (query) {
      for (const keyword of [...KEYWORDS, ...DIALECT_KEYWORDS[kind]]) {
        add({ label: keyword, kind: 'keyword', detail: '', usageKey: `k:${keyword}` }, 0);
      }
    }
  }

  scored.sort((a, b) => b.rank - a.rank || a.completion.label.localeCompare(b.completion.label));
  // The same column can come from two tables in scope; keep the best of each.
  const seen = new Set<string>();
  const result: Completion[] = [];
  for (const { completion } of scored) {
    const key = `${completion.kind}:${completion.label}:${completion.detail}`;
    if (seen.has(key)) continue;
    seen.add(key);
    result.push(completion);
    if (result.length >= (options.limit ?? 60)) break;
  }
  return result.length > 0 ? { from, options: result } : null;
}
