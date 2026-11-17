import { describe, expect, it } from 'vitest';
import { parseConnectionUrl } from './connectionUrl';

describe('parseConnectionUrl', () => {
  it('reads a PostgreSQL URL, keeping the password out of the config', () => {
    const parsed = parseConnectionUrl(
      'postgres://alice:p%40ss%2Fword@db.example.com:6432/shop?sslmode=verify-full',
    )!;
    expect(parsed.config).toMatchObject({
      kind: 'postgres',
      host: 'db.example.com',
      port: 6432,
      database: 'shop',
      user: 'alice',
    });
    expect(parsed.config.tls?.mode).toBe('verify-full');
    expect(parsed.password).toBe('p@ss/word');
    expect(JSON.stringify(parsed.config)).not.toContain('p@ss');
  });

  it('fills PostgreSQL defaults', () => {
    const parsed = parseConnectionUrl('postgresql://localhost')!;
    expect(parsed.config).toMatchObject({ kind: 'postgres', host: 'localhost', port: 5432 });
    expect(parsed.password).toBeUndefined();
    expect(parsed.config.tls).toBeUndefined();
  });

  it('reads ClickHouse URLs in their common spellings', () => {
    expect(parseConnectionUrl('clickhouse://default@ch.local/analytics')!.config).toMatchObject({
      kind: 'clickhouse',
      host: 'ch.local',
      port: 8123,
      database: 'analytics',
      user: 'default',
      tls: { mode: 'disable' },
    });
    expect(parseConnectionUrl('https://abc.clickhouse.cloud:8443')!.config).toMatchObject({
      kind: 'clickhouse',
      port: 8443,
      tls: { mode: 'verify-full' },
    });
    expect(parseConnectionUrl('https://abc.clickhouse.cloud')!.config.port).toBe(8443);
    // Native-protocol ports map to the HTTP interface Rowboat uses.
    expect(parseConnectionUrl('clickhouse://h:9000/db')!.config.port).toBe(8123);
    expect(parseConnectionUrl('clickhouse://h:9440/db')!.config).toMatchObject({
      port: 8443,
      tls: { mode: 'verify-full' },
    });
    expect(parseConnectionUrl('clickhouse://h?secure=true&database=x')!.config).toMatchObject({
      port: 8443,
      database: 'x',
    });
  });

  it('treats file paths as SQLite', () => {
    expect(parseConnectionUrl('/var/data/app.db')!.config).toEqual({
      kind: 'sqlite',
      path: '/var/data/app.db',
    });
    expect(parseConnectionUrl('C:\\data\\app.sqlite')!.config.path).toBe('C:\\data\\app.sqlite');
    expect(parseConnectionUrl('notes.sqlite3')!.config.kind).toBe('sqlite');
    expect(parseConnectionUrl('sqlite:///tmp/my%20db.sqlite')!.config.path).toBe(
      '/tmp/my db.sqlite',
    );
  });

  it('handles IPv6 hosts', () => {
    expect(parseConnectionUrl('postgres://u@[::1]:5433/db')!.config).toMatchObject({
      host: '::1',
      port: 5433,
    });
  });

  it('rejects what it does not understand', () => {
    for (const text of ['', '   ', 'hello world', 'mysql://root@localhost/db', 'postgres://']) {
      expect(parseConnectionUrl(text), text).toBeNull();
    }
  });
});
