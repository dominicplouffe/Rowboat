import { describe, expect, it } from 'vitest';
import { aliasFor, buildIndex, complete, fuzzyScore, quote, resolveScope } from './complete';
import type { SchemaSnapshot, SnapshotTable } from './types';

const table = (
  schema: string,
  name: string,
  columns: string[],
  foreignKeys: SnapshotTable['foreignKeys'] = [],
  kind: 'table' | 'view' = 'table',
): SnapshotTable => ({
  schema,
  name,
  kind,
  columns: columns.map((c) => {
    const [column, typeName = 'text'] = c.split(':');
    return { name: column, typeName, primaryKey: column === 'id' };
  }),
  foreignKeys,
});

const snapshot: SchemaSnapshot = {
  searchPath: ['public'],
  tables: [
    table('public', 'customers', ['id:integer', 'name', 'email', 'created_at:timestamptz']),
    table(
      'public',
      'orders',
      ['id:integer', 'customer_id:integer', 'total:numeric', 'status'],
      [
        {
          columns: ['customer_id'],
          refSchema: 'public',
          refTable: 'customers',
          refColumns: ['id'],
        },
      ],
    ),
    table(
      'public',
      'order_items',
      ['id:integer', 'order_id:integer', 'product_id:integer', 'qty:integer'],
      [{ columns: ['order_id'], refSchema: 'public', refTable: 'orders', refColumns: ['id'] }],
    ),
    table('public', 'order_customers', ['order_id:integer', 'customer_id:integer']),
    table('public', 'Mixed Case', ['id:integer', 'Odd Name']),
    table('archive', 'old_orders', ['id:integer', 'total:numeric']),
    table('public', 'big_orders', ['id:integer'], [], 'view'),
  ],
  functions: [
    { name: 'count', signature: '(*)' },
    { name: 'coalesce', signature: '(value, ...)' },
    { name: 'lower', signature: '(text)' },
    { name: 'length', signature: '(text)' },
  ],
};
const index = buildIndex('postgres', snapshot);

/** Completes at the `|` marker; returns labels in ranked order. */
function at(sql: string, usage?: Record<string, number>) {
  const cursor = sql.indexOf('|');
  const text = sql.replace('|', '');
  const result = complete(index, text, cursor, { usage });
  return {
    labels: result?.options.map((o) => o.label) ?? [],
    options: result?.options ?? [],
    replaced: result ? text.slice(result.from, cursor) : '',
  };
}

describe('context', () => {
  it('after FROM: tables and views, not columns', () => {
    const { labels, options } = at('select * from |');
    expect(labels).toContain('customers');
    expect(labels).toContain('big_orders');
    expect(labels).not.toContain('email');
    expect(options.find((o) => o.label === 'big_orders')?.kind).toBe('view');
  });

  it('tables outside the search path are offered schema-qualified', () => {
    const { options } = at('select * from old|');
    const old = options.find((o) => o.label === 'archive.old_orders');
    expect(old?.apply).toBe('archive.old_orders');
  });

  it('after "schema.": that schema’s objects only', () => {
    const { labels } = at('select * from archive.|');
    expect(labels).toEqual(['old_orders']);
  });

  it('in SELECT and WHERE: columns of the tables in scope come first', () => {
    const select = at('select | from orders');
    expect(select.labels.slice(0, 4).sort()).toEqual(['customer_id', 'id', 'status', 'total']);
    expect(select.labels).not.toContain('email');

    const where = at('select * from customers where em|');
    expect(where.labels[0]).toBe('email');
    expect(where.replaced).toBe('em');
  });

  it('shows the column type and its table', () => {
    const { options } = at('select tot| from orders');
    expect(options[0]).toMatchObject({
      label: 'total',
      kind: 'column',
      detail: 'numeric · orders',
    });
  });

  it('completes the statement under the cursor, not its neighbours', () => {
    const { labels } = at(
      'select * from customers; select | from orders; select * from order_items',
    );
    expect(labels).toContain('status');
    expect(labels).not.toContain('email');
    expect(labels).not.toContain('qty');
  });

  it('ignores keywords inside strings and comments', () => {
    const { labels } = at("select 'x from customers', | from orders -- from order_items");
    expect(labels).toContain('status');
    expect(labels).not.toContain('email');
    expect(labels).not.toContain('qty');
  });
});

describe('aliases, CTEs and subqueries', () => {
  it('after "alias.": that alias’s columns', () => {
    const { labels } = at('select o.| from orders o join customers c on c.id = o.customer_id');
    expect(labels.sort()).toEqual(['customer_id', 'id', 'status', 'total']);
    expect(at('select c.na| from orders as o, customers as c').labels).toEqual(['name']);
  });

  it('offers the aliases themselves', () => {
    const { options } = at('select | from orders o join customers c on true');
    expect(
      options
        .filter((o) => o.kind === 'alias')
        .map((o) => o.label)
        .sort(),
    ).toEqual(['c', 'o']);
  });

  it('resolves CTE columns from their select list or explicit column list', () => {
    const sql =
      'with recent as (select id, total as amount, lower(status) from orders), named (a, b) as (select 1, 2) select r.| from recent r, named';
    // An unaliased function call is named after the function, as PostgreSQL does.
    expect(at(sql).labels.sort()).toEqual(['amount', 'id', 'lower']);
    expect(at(sql.replace('r.|', 'named.|')).labels.sort()).toEqual(['a', 'b']);
  });

  it('resolves subquery columns', () => {
    const { labels } = at(
      'select s.| from (select customer_id, count(*) as n from orders group by 1) s',
    );
    expect(labels.sort()).toEqual(['customer_id', 'n']);
  });

  it('lists what is in scope', () => {
    const scope = resolveScope(
      index,
      'update orders set status = 1 where id in (select order_id from order_items oi)',
    );
    expect(scope.map((s) => [s.name, s.table?.name])).toEqual([
      ['orders', 'orders'],
      ['oi', 'order_items'],
    ]);
  });
});

describe('join completion', () => {
  it('offers whole clauses built from foreign keys, in both directions', () => {
    const { options } = at('select * from customers c join |');
    const joins = options.filter((o) => o.kind === 'join').map((o) => o.label);
    expect(joins).toEqual(['orders o ON o.customer_id = c.id']);
    // Joins rank above plain table names.
    expect(options[0].kind).toBe('join');

    const reverse = at('select * from orders o join |')
      .options.filter((o) => o.kind === 'join')
      .map((o) => o.label);
    expect(reverse.sort()).toEqual([
      'customers c ON c.id = o.customer_id',
      'order_items oi ON oi.order_id = o.id',
    ]);
  });

  it('picks an alias that is not taken', () => {
    const { options } = at('select * from customers o join |');
    expect(options.find((o) => o.kind === 'join')?.label).toBe(
      'orders o2 ON o2.customer_id = o.id',
    );
    expect(aliasFor('order_items', new Set())).toBe('oi');
    expect(aliasFor('OrderItems', new Set(['oi']))).toBe('oi2');
    expect(aliasFor('as', new Set())).not.toBe('as');
  });
});

describe('matching and ranking', () => {
  it('fuzzy-matches across word boundaries', () => {
    expect(at('select * from ordcust|').labels[0]).toBe('order_customers');
    expect(fuzzyScore('xyz', 'orders')).toBeNull();
    expect(fuzzyScore('ord', 'orders')!).toBeGreaterThan(fuzzyScore('ord', 'old_orders')!);
  });

  it('ranks by scope first, then by how often you use something', () => {
    // "status" (in scope) beats the function and keyword matching "st".
    expect(at('select st| from orders').labels[0]).toBe('status');
    // Among equals, past use decides.
    const plain = at('select * from o|').labels;
    expect(plain.indexOf('orders')).toBeLessThan(plain.indexOf('order_items'));
    const used = at('select * from o|', { 't:public.order_items': 9 }).labels;
    expect(used[0]).toBe('order_items');
  });

  it('offers functions with their argument hint and keywords once typing starts', () => {
    const { options } = at('select coa| from orders');
    expect(options[0]).toMatchObject({
      label: 'coalesce',
      kind: 'function',
      detail: '(value, ...)',
      apply: 'coalesce(',
    });
    expect(at('select * from orders wh|').labels).toContain('WHERE');
    expect(at('select | from orders').options.some((o) => o.kind === 'function')).toBe(false);
  });

  it('quotes names that need it when inserting', () => {
    const { options } = at('select * from mix|');
    expect(options[0]).toMatchObject({ label: 'Mixed Case', apply: '"Mixed Case"' });
    expect(
      at('select m.| from "Mixed Case" m').options.find((o) => o.label === 'Odd Name')?.apply,
    ).toBe('"Odd Name"');
    expect(quote('postgres', 'order')).toBe('"order"');
    expect(quote('clickhouse', 'my-col')).toBe('`my-col`');
    expect(quote('clickhouse', 'UserId')).toBe('UserId');
    expect(quote('postgres', 'UserId')).toBe('"UserId"');
  });

  it('returns nothing rather than noise when nothing matches', () => {
    expect(complete(index, 'select zzzqqq', 13)).toBeNull();
    expect(complete(index, 'select o. from orders o', 9)?.options.length).toBe(4);
    expect(complete(index, 'select nope. from orders o', 12)).toBeNull();
  });
});

describe('performance', () => {
  it('stays under 30 ms per keystroke on a 500-table schema', () => {
    const big: SchemaSnapshot = {
      searchPath: ['public'],
      tables: Array.from({ length: 500 }, (_, t) =>
        table(
          'public',
          `table_${t}_of_things`,
          Array.from({ length: 20 }, (_, c) => `column_${c}_x:integer`),
          t > 0
            ? [
                {
                  columns: ['column_1_x'],
                  refSchema: 'public',
                  refTable: `table_${t - 1}_of_things`,
                  refColumns: ['column_0_x'],
                },
              ]
            : [],
        ),
      ),
      functions: Array.from({ length: 2000 }, (_, f) => ({
        name: `function_${f}`,
        signature: '(x)',
      })),
    };
    const bigIndex = buildIndex('postgres', big);
    const sql = `with recent as (select column_1_x from table_3_of_things)
      select a.column_2_x, b.col from table_10_of_things a
      join table_11_of_things b on b.column_1_x = a.column_0_x
      where a.col`;
    const cases: [string, number][] = [
      [sql, sql.length],
      ['select * from tab', 17],
      ['select * from table_10_of_things a join ', 40],
      ['select fun from table_250_of_things', 10],
    ];
    const timings: number[] = [];
    for (let round = 0; round < 25; round++) {
      for (const [text, cursor] of cases) {
        const started = performance.now();
        complete(bigIndex, text, cursor);
        timings.push(performance.now() - started);
      }
    }
    timings.sort((a, b) => a - b);
    const p95 = timings[Math.floor(timings.length * 0.95)];
    expect(p95).toBeLessThan(30);
  });
});
