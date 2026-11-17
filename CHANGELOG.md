# Changelog

## 0.1.0 (unreleased)

The first version. Not yet released: see [docs/RELEASING.md](docs/RELEASING.md)
for what has to happen first.

### Connections

- PostgreSQL, ClickHouse (over HTTP or HTTPS) and SQLite.
- TLS from "disable" to "verify-full", client certificates, SSH tunnels with
  password, key or agent, and known-hosts checking.
- Passwords and passphrases in the OS keychain only.
- Prod connections ask before destructive statements; read-only connections.
- Your Client ID is sent as `application_name` (PostgreSQL) and as the client
  name (ClickHouse).

### Editor

- Schema-aware autocomplete with aliases, CTEs and foreign-key joins.
- Run a statement, a selection or the whole script; cancel within a second.
- Formatter, EXPLAIN as a tree, query history, `.sql` files, restored tabs.
- Per-connection auto-commit with Commit and Rollback.

### Results

- Row limit (default 1,000) applied while fetching, never by rewriting SQL.
- Next page, fetch all (up to the memory cap) and count rows.
- Sort, filter, resize and hide columns; record view; JSON viewer.
- Inline editing with a preview of the exact UPDATE statements.
- Export to CSV, TSV, JSON, NDJSON, SQL INSERT and Markdown, streamed to disk.

### AI copilot

- Ask (natural language to SQL), Fix my query (a diff under the error) and
  Explain, with OpenAI, Anthropic, Gemini, DeepSeek, Mistral, xAI, OpenRouter,
  Azure OpenAI, local Ollama / LM Studio, or any OpenAI-compatible endpoint.
- The copilot never runs SQL and never sends result rows. "View context" shows
  exactly what was sent. AI can be turned off per connection.

### App

- First-launch setup with a sample database.
- Searchable settings, rebindable shortcuts, light and dark themes.
- Update notice (can be turned off). No telemetry and no crash reports.

### Known limits

- Builds are unsigned.
- ClickHouse support has not been run against a real server yet.
- The copilot has not been run against real providers yet.
- macOS and Windows builds have not been produced yet.
