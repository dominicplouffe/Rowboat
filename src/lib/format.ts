// SQL formatting (Cmd/Ctrl+Shift+F). The formatter is loaded on first use.

import type { DbKind, Settings } from './types';

const LANGUAGE: Record<DbKind, string> = {
  postgres: 'postgresql',
  sqlite: 'sqlite',
  clickhouse: 'clickhouse',
};

export async function formatSql(
  sql: string,
  kind: DbKind | null,
  editor: Settings['editor'],
): Promise<string> {
  const { format, supportedDialects } = await import('sql-formatter');
  const wanted = kind ? LANGUAGE[kind] : 'sql';
  return format(sql, {
    // Fall back to standard SQL if this build has no dialect for the database.
    language: (supportedDialects.includes(wanted) ? wanted : 'sql') as never,
    keywordCase: editor.formatterKeywordCase,
    tabWidth: editor.tabSize,
    useTabs: editor.indentWithTabs,
  });
}
