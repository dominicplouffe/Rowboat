// "Paste URL": turns a connection string into form fields.
//
// Runs in the UI because the text comes straight from the user's clipboard;
// a password found in it goes into the password field like any typed one and
// is sent to the core once, on save.

import type { ConnectionConfig, DbKind, TlsMode } from './types';

export interface ParsedUrl {
  config: Partial<ConnectionConfig> & { kind: DbKind };
  password?: string;
}

const TLS_MODES: TlsMode[] = ['disable', 'prefer', 'require', 'verify-ca', 'verify-full'];

function decode(text: string): string {
  try {
    return decodeURIComponent(text);
  } catch {
    return text;
  }
}

/** Returns null when the text is not a connection string Rowboat understands. */
export function parseConnectionUrl(input: string): ParsedUrl | null {
  const text = input.trim();
  if (!text) return null;

  const scheme = /^([a-z][a-z0-9+.-]*):\/\//i.exec(text)?.[1]?.toLowerCase();
  if (!scheme) {
    // No scheme: a file path means SQLite.
    const looksLikePath =
      /^(\/|~\/|\.{1,2}[\\/]|[a-zA-Z]:[\\/])/.test(text) || /\.(sqlite3?|db3?)$/i.test(text);
    return looksLikePath ? { config: { kind: 'sqlite', path: text } } : null;
  }
  if (scheme === 'sqlite' || scheme === 'file') {
    const path = decode(text.replace(/^[a-z]+:\/\//i, '').replace(/\?.*$/, ''));
    return path ? { config: { kind: 'sqlite', path } } : null;
  }

  const kind: DbKind | null =
    scheme === 'postgres' || scheme === 'postgresql'
      ? 'postgres'
      : scheme === 'clickhouse' ||
          scheme === 'clickhouses' ||
          scheme === 'http' ||
          scheme === 'https'
        ? 'clickhouse'
        : null;
  if (!kind) return null;

  let url: URL;
  try {
    // The URL parser only fills in host and port for schemes it knows.
    url = new URL(text.replace(/^[a-z][a-z0-9+.-]*:/i, 'http:'));
  } catch {
    return null;
  }
  if (!url.hostname) return null;

  const config: ParsedUrl['config'] = {
    kind,
    host: url.hostname.replace(/^\[|\]$/g, ''),
    database: decode(url.pathname.replace(/^\//, '')),
    user: decode(url.username),
  };
  const password = url.password ? decode(url.password) : undefined;
  const params = url.searchParams;

  if (kind === 'postgres') {
    config.port = url.port ? Number(url.port) : 5432;
    const mode = params.get('sslmode') as TlsMode | null;
    if (mode && TLS_MODES.includes(mode)) {
      config.tls = {
        mode,
        caCertPath: params.get('sslrootcert') ?? '',
        clientCertPath: params.get('sslcert') ?? '',
        clientKeyPath: params.get('sslkey') ?? '',
      };
    }
  } else {
    const secure =
      scheme === 'https' ||
      scheme === 'clickhouses' ||
      params.get('secure') === 'true' ||
      params.get('secure') === '1' ||
      url.port === '8443' ||
      url.port === '9440';
    // Native-protocol ports in a pasted URL map to their HTTP equivalents.
    const port = url.port === '9000' ? 8123 : url.port === '9440' ? 8443 : Number(url.port);
    config.port = port || (secure ? 8443 : 8123);
    config.tls = {
      mode: secure ? 'verify-full' : 'disable',
      caCertPath: '',
      clientCertPath: '',
      clientKeyPath: '',
    };
    if (!config.database && params.get('database')) config.database = params.get('database')!;
  }
  return { config, password };
}
