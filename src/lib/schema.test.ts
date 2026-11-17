import { describe, expect, it } from 'vitest';
import { insertTemplate, qualifiedName, quoteIdent, selectTemplate } from './schema.svelte';

describe('navigator SQL helpers', () => {
  it('quotes identifiers only when needed, per database', () => {
    expect(quoteIdent('postgres', 'orders')).toBe('orders');
    expect(quoteIdent('postgres', 'Order Items')).toBe('"Order Items"');
    expect(quoteIdent('postgres', 'say"hi')).toBe('"say""hi"');
    expect(quoteIdent('sqlite', '2024')).toBe('"2024"');
    expect(quoteIdent('clickhouse', 'events')).toBe('events');
    expect(quoteIdent('clickhouse', 'my-table')).toBe('`my-table`');
    expect(quoteIdent('clickhouse', 'a`b')).toBe('`a\\`b`');
  });

  it('builds qualified names from navigator paths', () => {
    expect(qualifiedName('postgres', ['public', 'Tables', 'orders'])).toBe('public.orders');
    expect(qualifiedName('postgres', ['Sales', 'Views', 'big orders'])).toBe(
      '"Sales"."big orders"',
    );
    expect(qualifiedName('clickhouse', ['analytics', 'Tables', 'events'])).toBe('analytics.events');
    expect(qualifiedName('sqlite', ['Tables', 'notes'])).toBe('notes');
  });

  it('generates SELECT and INSERT templates', () => {
    const path = ['public', 'Tables', 'orders'];
    expect(selectTemplate('postgres', path, ['id', 'Total'])).toBe(
      'SELECT id,\n       "Total"\nFROM public.orders;\n',
    );
    expect(selectTemplate('postgres', path, [])).toBe('SELECT *\nFROM public.orders;\n');
    expect(insertTemplate('postgres', path, ['id', 'total'])).toBe(
      'INSERT INTO public.orders (id, total)\nVALUES (/* id */ NULL,\n        /* total */ NULL);\n',
    );
  });
});
