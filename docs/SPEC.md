# Rowboat — Product Spec v0.1

Nov 17, 2026

## Overview

Rowboat is a fast, lightweight SQL client for macOS, Windows and Linux with an AI copilot that knows your schema. v0.1 ships PostgreSQL, ClickHouse and SQLite, a first-class editor and data grid, and nothing that slows startup. It is MIT licensed and built for developers, data analysts and DBAs.

Repository: [github.com/dominicplouffe/Rowboat](https://github.com/dominicplouffe/Rowboat). The name is one word: **Row**boat, because every query brings back rows.

Product principles:

- **Fast first.** Cold start under 1 s, idle memory under 150 MB. Anything not needed at launch loads lazily.
- **Keyboard-first, mouse-friendly.** Every action has a shortcut; nothing requires one.
- **AI is a copilot, not an autopilot.** AI-generated SQL never runs until you press Run.
- **Safe by default.** Secrets live in the OS keychain; production connections are guarded.
- **Bring your own model.** Any major LLM provider, or a local model with no key at all.

## v0.1 scope

v0.1 is a sharp SQL client for three databases with a schema-aware AI. ERDs, data migration and NoSQL wait until the core is fast and stable.

| Area | In v0.1 | Later |
| --- | --- | --- |
| Databases | PostgreSQL, ClickHouse, SQLite | MySQL/MariaDB, SQL Server, DuckDB, Oracle, MongoDB, Redis, cloud warehouses |
| Editor | Full syntax highlighting, schema-aware autocomplete, formatter, tabs, run statement / selection / script, cancel, EXPLAIN plan tree | Snippets, vim keymap, query variables, AI ghost-text completion |
| Data | Virtualized grid, row limits, inline editing, filter/sort, JSON viewer, record view, export | Import wizard, cross-database transfer, charts/pivots |
| Schema | Navigator tree, DDL view, columns / indexes / constraints | ER diagrams, schema compare |
| Connectivity | SSL/TLS, SSH tunnel (password, key, agent) | HTTP proxy, Kerberos, cloud IAM auth |
| AI | Text-to-SQL, fix-on-error, explain query, all major providers + local | Query optimization, multi-step agent, embeddings index |
| Platform | Onboarding, settings, query history, session restore, auto-update, light/dark | Plugins, team-shared connections, settings sync |

## First-launch onboarding

A four-step wizard runs on first launch. Only the Client ID is required; every other step can be skipped and changed later in Settings.

1. **Client ID.** Pick the identity Rowboat uses for your sessions. Prefilled as `rowboat-<os username>`. It is sent as `application_name` on PostgreSQL and as the client name / `log_comment` on ClickHouse, so DBAs can see who is running what.
2. **AI provider.** Choose a provider, paste the API key, pick a default model from the provider's live model list, then press **Test** (a one-token call). Options for "Local (Ollama / LM Studio)" and "Skip — no AI".
3. **Look and feel.** Light / dark / system theme, editor font size, default row limit.
4. **First connection.** Add a PostgreSQL, ClickHouse or SQLite connection, or open a small bundled SQLite sample database to try things out.

The wizard is re-runnable from Settings → General.

## Connections

Connections live in a sidebar tree with folders, colors and an environment tag (Dev / Staging / Prod), and each opens a lazily loaded schema navigator.

| Option | PostgreSQL | ClickHouse | SQLite |
| --- | --- | --- | --- |
| Address | Host, port 5432 | Host, HTTP 8123 / HTTPS 8443 (native 9000 later) | File path; create new file |
| Auth | User/password, .pgpass | User/password | — |
| TLS | disable → verify-full, CA + client cert | HTTPS + custom CA | — |
| SSH tunnel | Yes | Yes | — |
| Paste URL | `postgres://…` | `clickhouse://…`, `https://…` | File path |
| Read-only mode | `default_transaction_read_only` | `readonly=1` | Open with `mode=ro` |
| Transactions | Auto-commit or manual Commit/Rollback | Auto-commit only | Auto-commit or manual |
| Navigator shows | Schemas, tables, views, mat. views, functions, sequences, types | Databases, tables (with engine, ORDER BY, partition key), views, dictionaries | Tables, views, indexes, triggers |

Also in v0.1:

- **Prod guard.** Prod-tagged connections get a red accent and a confirmation before DROP, TRUNCATE, ALTER, or UPDATE/DELETE without WHERE. Read-only can be the default for Prod.
- **Reliability.** Test button, connect and query timeouts, keepalive, automatic reconnect with the tab state kept.
- **Navigator actions.** Filter, Show DDL, Select top N, Copy name, Refresh, Generate SELECT/INSERT template.

## SQL editor & autocomplete

The editor is CodeMirror 6 with dialect-aware highlighting and schema-aware autocomplete — the feature users will judge Rowboat by.

**Highlighting and editing**

- Dialects: PostgreSQL (incl. `$$` dollar-quoting, JSON operators), SQLite, and a custom ClickHouse dialect (its types, functions, `FORMAT`, `SETTINGS`, `ARRAY JOIN`). JSON highlighting in the JSON viewer.
- Bracket matching, multi-cursor, folding, regex find/replace, comment toggle, SQL formatter (Cmd/Ctrl+Shift+F).
- Inline error markers: the database's error position becomes a squiggle at the right line and column.

**Running queries**

- Cmd/Ctrl+Enter runs the statement under the cursor; a selection runs the selection; Cmd/Ctrl+Shift+Enter runs the whole script.
- A dialect-aware statement splitter handles strings, comments and dollar-quoting.
- Cancel any running query (PostgreSQL cancel request, ClickHouse `KILL QUERY` by query ID, SQLite interrupt).
- Scripts produce one result tab per statement plus an execution log (duration, rows, errors).
- Query history per connection, searchable. Saved scripts are plain `.sql` files on disk.
- Session restore: open tabs and unsaved text survive restarts and crashes.

**Autocomplete**

- **Context-aware.** After FROM/JOIN: tables and views. After `alias.`: that alias's columns. In SELECT/WHERE: columns of the tables in scope. After `schema.`: that schema's objects.
- **Alias- and CTE-aware.** Resolves aliases, CTEs and subquery columns in the current statement.
- **Join completion.** After JOIN, offers whole clauses built from foreign keys, e.g. `orders o ON o.customer_id = c.id`.
- **Functions with signatures.** Dialect keywords and functions with argument hints; ClickHouse functions loaded from `system.functions` on connect.
- **Fuzzy and ranked.** `ordcust` matches `order_customers`; ranked by scope, then by how often you use it.
- **Instant.** Served from an in-memory schema cache, under 30 ms per keystroke; refreshed on demand and after DDL.

## Results, row limits & JSON

Results stream into a virtualized grid, and only a bounded page is ever fetched, so a `SELECT *` on a billion-row table stays instant and cheap.

**Row limit**

- Default 1,000 rows per fetch. Change it in one click from the limit dropdown in the results toolbar (100 / 500 / 1,000 / 5,000 / custom / no limit), per tab; the default lives in Settings.
- The limit is applied at fetch time, not by rewriting your SQL: PostgreSQL uses a server-side cursor; ClickHouse uses `max_result_rows` with `result_overflow_mode=break`; SQLite stops stepping.
- The status bar shows "1,000 rows · more available" with **Fetch next page**, **Fetch all** (with a warning above a memory cap, default 256 MB) and **Count rows**.

**Grid**

- Sort, filter, resize, reorder and hide columns; copy as TSV, CSV, JSON, Markdown or INSERT statements.
- Type-aware cells: NULL shown distinctly, numbers right-aligned, dates in your chosen time zone, binary as hex.
- Record view: one row as a form, for wide tables.
- **Inline editing** when the result is a single table with a primary key (or SQLite rowid). Edits are staged and highlighted, then applied in one transaction with **Preview SQL**. ClickHouse grids are read-only in v0.1.

**JSON viewer**

- Opens for `json`/`jsonb` (PostgreSQL), JSON columns (ClickHouse), and any text cell that parses as JSON.
- Side panel with pretty-printed, highlighted, collapsible tree; search; copy value or JSON path; raw/formatted toggle.

**Export** — CSV, TSV, JSON, NDJSON, SQL INSERT, Markdown. Current page, or the full result streamed straight to a file without loading it into memory.

## AI assistant

The AI writes, fixes and explains SQL from a compact local index of your schema. It never runs anything itself, and it sends no row data unless you allow it.

**Providers.** Most providers speak the OpenAI-compatible API, so v0.1 needs only three adapters. Model lists come from each provider's models endpoint rather than being hard-coded.

| Provider | Adapter | Key needed |
| --- | --- | --- |
| OpenAI | OpenAI | Yes |
| Anthropic (Claude) | Native Messages API | Yes |
| Google Gemini | Native | Yes |
| DeepSeek | OpenAI-compatible | Yes |
| Mistral | OpenAI-compatible | Yes |
| xAI (Grok) | OpenAI-compatible | Yes |
| OpenRouter | OpenAI-compatible | Yes |
| Azure OpenAI | OpenAI-compatible + deployment name | Yes |
| Ollama / LM Studio (local) | OpenAI-compatible | No |
| Custom endpoint | OpenAI-compatible, any base URL | Optional |

**Schema index**

- Built in the background on connect: tables, views, columns and types, nullability, primary and foreign keys, indexes, comments, enum values, row-count estimates, and ClickHouse engine / ORDER BY / partition keys.
- Stored locally per connection with a fingerprint, so refreshes are incremental. Manual **Re-index** available.
- Sent in a compact form, e.g. `orders(id int pk, customer_id int fk→customers.id, status text, created_at timestamptz)`.
- Small schemas go in whole. Large ones are ranked by name/comment match to the question plus query-history usage, expanded along foreign keys, and capped by a token budget.
- Optional sample values (a few distinct values per low-cardinality column) help with WHERE filters. Off by default, per connection.

**Features**

- **Ask (Cmd/Ctrl+K or side panel).** Natural language → SQL in the active connection's dialect and server version. Insert, replace selection, or copy. A `-- comment` above the cursor works as a prompt too.
- **Fix my query.** When a query fails with a syntax or schema error, a suggested fix appears automatically under the error, shown as a diff with a one-line explanation. **Accept** or **Dismiss**. Network and permission errors are not sent. Can be switched to manual.
- **Explain query.** Plain-English explanation of the selected SQL.

**Guardrails**

- Generated SQL is parsed before it is shown; a parse failure triggers one automatic retry with the parser error.
- Destructive statements in AI output get a warning badge.
- **View context** shows the exact payload sent. AI can be disabled per connection.
- Responses stream, can be cancelled, and show token usage.

## Settings

Settings is a searchable window with nine sections; secrets are stored in the OS keychain, everything else in a plain settings file that can be exported without secrets.

| Section | Settings |
| --- | --- |
| General | Client ID, restore tabs on start, update checks (auto / notify / off), update channel (stable / beta), re-run onboarding |
| Appearance | Theme (light / dark / system), UI density, editor and grid fonts and sizes |
| Editor | Tab size, spaces vs tabs, word wrap, line numbers, auto-close brackets and quotes, autocomplete on typing vs on demand, formatter keyword case |
| Execution | Default row limit, query timeout, auto-commit default, stop script on error, confirm destructive statements (always / Prod only / never), result memory cap |
| Results | NULL display text, date/time format and time zone (local / UTC), number format, binary display (hex / base64), max characters per cell, JSON auto-detect |
| Connections | Connect timeout, keepalive interval, auto-reconnect, SSH known_hosts policy |
| AI | Providers and keys (several, one default), model per task (e.g. a cheaper model for fixes), auto-suggest fixes, send sample values, schema token budget, base URL override, request timeout |
| Keyboard | View and rebind every shortcut |
| Privacy & data | Keep query history (on / off), retention in days, clear history, export / import settings, open data folder |

## Performance budget

These targets are enforced in CI, so a regression fails the build rather than reaching users.

| Metric | Target | How |
| --- | --- | --- |
| Cold start to usable window | < 1 s (2021+ laptop) | Native Tauri shell, no splash; editor languages and AI code load lazily |
| Idle memory, app + webview | < 150 MB (macOS/Linux), < 200 MB (Windows) | System webview instead of bundled Chromium; no background polling |
| Installer size | < 20 MB | Rust binary + small frontend |
| Initial JS bundle | < 400 KB gzipped | Svelte, code-split CodeMirror languages, no UI kit |
| Autocomplete latency | < 30 ms p95 | In-memory schema cache in the frontend |
| Grid scrolling | 60 fps with 100k rows loaded | Virtualized rows, recycled DOM nodes |
| Result memory | Capped, default 256 MB | Paged fetch; exports stream to disk |

Windows gets a higher memory target because WebView2 runs extra helper processes.

## Architecture & tech stack

Rowboat is a Tauri 2 app: a Rust core does all database, AI and secret handling, and a thin Svelte UI renders in the system webview. Tauri 2 is mature and actively released (2.12 is current), and it avoids shipping Chromium, which is what makes the memory and size targets reachable.

```text
┌─ Svelte UI in the system webview ────────────────────────────────────┐
│  Editor (CodeMirror 6) · Results grid · Navigator · AI panel · Settings │
└───────────────────────────────┬───────────────────────────────────────┘
                                │ Tauri IPC: commands + streaming channels
┌─ Rust core (Tauri 2) ─────────┴───────────────────────────────────────┐
│  Query engine ──┐   Schema index ──► AI service        Secrets         │
│  (split, page,  │   (introspect,     (prompt,          (keys stay      │
│   cancel)       │    rank)            validate SQL)     in Rust)        │
│        ▼        │        ▼                 ▼                │          │
│  DB drivers     │   App storage       LLM adapters          │          │
│  (Postgres,     │   (history,         (OpenAI-compatible,   │          │
│   ClickHouse,   │    settings,         Anthropic, Gemini)   │          │
│   SQLite)       │    schema cache)          │               │          │
└────────┬────────┴───────────────────────────┼───────────────┼──────────┘
         ▼                                    ▼               ▼
   Databases (direct or SSH tunnel)     LLM providers     OS keychain
                                        (cloud or Ollama)
```

The webview only renders. It reaches databases, LLMs and the keychain through the Rust core over Tauri IPC.

| Layer | Choice | Why |
| --- | --- | --- |
| Shell | Tauri 2 (Rust) | Uses WKWebView / WebView2 / WebKitGTK; ~10× smaller than Electron |
| UI | Svelte 5 + TypeScript + Vite | Tiny runtime, fast startup |
| Editor | CodeMirror 6 (`lang-sql`, `lang-json`, custom ClickHouse dialect) | Modular and light; Monaco is several MB heavier |
| Grid | Custom virtualized grid (evaluate RevoGrid first) | Full control over paging, editing, JSON cells |
| PostgreSQL | `tokio-postgres` + rustls | Async, cancel requests, cursors |
| ClickHouse | HTTP interface via `reqwest`, `RowBinaryWithNamesAndTypes` streaming | Binary rows with keep-alive and compression: no text parsing, works with ClickHouse Cloud and proxies |
| SQLite | `rusqlite` (bundled SQLite) | Same SQLite version and JSON support everywhere |
| SQL parsing | `sqlparser-rs` (PostgreSQL, ClickHouse, SQLite dialects) | Statement splitting, AI output validation, destructive-statement detection |
| SSH | `russh` | Pure Rust, no OpenSSH dependency |
| Secrets | `keyring` crate | macOS Keychain, Windows Credential Manager, Linux Secret Service; encrypted-file fallback |
| App storage | SQLite file | Connections, history, schema index |
| LLM calls | `reqwest` + SSE streaming, in Rust | API keys never reach the webview |
| Updates | `tauri-plugin-updater` | Signed updates from GitHub Releases |

Repository layout:

```text
Rowboat/
  src/                 Svelte UI (editor, grid, navigator, ai, settings, onboarding)
  src-tauri/
    src/db/            driver trait + postgres.rs, clickhouse.rs, sqlite.rs
    src/ai/            providers (openai_compat, anthropic, gemini), schema_index.rs
    src/               secrets.rs, ssh.rs, settings.rs, history.rs
    icons/             generated app icons
  assets/brand/        logo source SVGs, wordmark, social preview
  packaging/arch/      PKGBUILD
  .github/workflows/   ci.yml, release.yml
  LICENSE              MIT
```

## Security & privacy

No account, no telemetry or crash reporting of any kind, and no secret ever stored in plain text.

- Database passwords, SSH passphrases and LLM keys go to the OS keychain; they never appear in config files, logs or exports.
- LLM requests are made from the Rust core, so the webview never holds an API key after entry.
- AI receives schema metadata only. Sample values are opt-in per connection. Fix requests include your query text, which may contain literals — the AI settings say so plainly.
- SSH host keys are checked against known_hosts; TLS supports full certificate verification.
- Tauri capabilities expose only the commands the UI needs; strict CSP; the webview loads no remote content.
- Builds and auto-updates are signed.

## Branding & app assets

The logo is a small wooden rowboat seen from the side. Its hull is painted in horizontal stripes like rows in a results grid, and two oars dip into a single wave. At 16–32 px the oars and wave drop away and only the striped hull remains, so the icon stays readable in a taskbar.

- **Palette:** deep-sea navy `#0E2A47` (outlines, wordmark), ocean blue `#2D9CDB` (wave), warm wood `#E8A33D` with cream `#FFF4E0` stripes (hull).
- **Wordmark:** "Rowboat" as one word, with "Row" bold and "boat" regular so the pun reads at a glance. Rounded geometric open-source font (OFL-licensed).
- **Source of truth:** one 1024×1024 master SVG/PNG; `tauri icon` generates most platform files from it.
- **Identifiers:** app name `Rowboat`, binary `rowboat`, bundle ID `com.dominicplouffe.rowboat`.

| Asset | Sizes / format | Used by |
| --- | --- | --- |
| Master icon | SVG + 1024×1024 PNG | Input for `tauri icon` |
| macOS icon | `icon.icns` (16–1024 px, @1x and @2x) | App bundle, DMG |
| Windows icon | `icon.ico` (16, 24, 32, 48, 64, 256 px) | EXE, installer, taskbar |
| Windows tiles | Square30x30 → Square310x310, StoreLogo PNGs | MSIX / Start menu |
| Linux icons | PNG 32, 64, 128, 256, 512 px + scalable SVG; `.desktop` file | deb, rpm, AppImage, Arch |
| Small-size glyph | Simplified 16 / 32 px PNG | Favicon-size contexts |
| Installer art | DMG background 660×400 (@2x too); NSIS header 150×57, sidebar 164×314 | DMG, Windows installer |
| Repo / web | Logo + wordmark, light and dark (SVG, PNG); GitHub social preview 1280×640 | README, GitHub |
| In-app | SVG logo | Onboarding, About window |

## Build, CI & distribution

Two GitHub Actions workflows: `ci.yml` checks every push and PR, and `release.yml` builds installers for all platforms when a `v*` tag is pushed.

**ci.yml (push and PR)**

- Lint: `cargo fmt --check`, `cargo clippy -D warnings`, ESLint, Prettier, `svelte-check`.
- Test: `cargo test`, Vitest; integration tests against PostgreSQL and ClickHouse service containers (Linux runner), SQLite everywhere.
- Build check on macOS, Windows and Ubuntu; fail if bundle size or startup-time budgets are exceeded.

**release.yml (tag v\*) — uses `tauri-apps/tauri-action`, publishes a draft GitHub Release**

| Runner | Target | Artifacts |
| --- | --- | --- |
| `macos-latest` | `universal-apple-darwin` (Apple Silicon + Intel) | `.dmg`, updater `.app.tar.gz` |
| `windows-latest` | `x86_64-pc-windows-msvc` | `.msi`, NSIS `-setup.exe` |
| `ubuntu-22.04` | `x86_64-unknown-linux-gnu` | `.deb`, `.rpm`, `.AppImage` |
| `ubuntu-latest` + `archlinux:base-devel` container | x86_64 | `packaging/arch/PKGBUILD` validated with `makepkg`; published to the AUR as `rowboat-sql-bin` |

Ubuntu 22.04 is the Linux build base on purpose: its older glibc means the binaries run on more distributions.

**Secrets to configure in the repo (later — see Decisions)**

- macOS signing and notarization: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`.
- Updater signing: `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
- Windows code signing (optional at first): an Authenticode certificate or Azure Trusted Signing.

Unsigned builds work for early testing, but macOS Gatekeeper and Windows SmartScreen will warn users. Workflows must build cleanly when these secrets are absent.

## Milestones

v0.1 ships in about 12 weeks across five milestones (rough estimate for one full-time developer; each milestone ends with a runnable build).

| Milestone | Weeks | Delivers |
| --- | --- | --- |
| M0 Foundation | 1–2 | Tauri + Svelte app, CI on all 3 OSes, logo and icons, settings store |
| M1 Connections | 3–5 | 3 database drivers, SSL and SSH, keychain secrets, run and cancel |
| M2 Editor | 6–8 | Autocomplete, virtual grid, row limits, JSON viewer, history, export |
| M3 AI copilot | 9–10 | All LLM providers, schema index, fix on error, onboarding wizard |
| M4 Release | 11–12 | Perf budgets in CI, installers, README and docs, tag v0.1 |

Release gate: every CI budget green. M0 comes first on purpose: with CI building all platforms from day one, packaging problems surface in week 2 instead of week 12.

## v0.1 acceptance criteria

v0.1 is done when every box is ticked on macOS, Windows and Ubuntu.

- [ ] First launch sets the Client ID, and it shows as `application_name` in `pg_stat_activity`. AI, theme and row-limit steps can be skipped.
- [ ] Connects to PostgreSQL 13–18, current ClickHouse LTS and SQLite 3, with TLS and SSH tunnel. Secrets exist only in the OS keychain.
- [ ] Runs statement, selection and script; cancels a 60-second query within 1 s on each database.
- [ ] `SELECT *` on a 10M-row table shows the first 1,000 rows without fetching the rest; memory stays under the cap.
- [ ] Autocomplete resolves aliases, CTEs and FK join clauses, at < 30 ms p95 on a 500-table schema.
- [ ] JSON cells open formatted in the viewer; a 1M-row export streams to CSV under the memory cap.
- [ ] Inline edits on a primary-key table preview the exact UPDATE and apply in one transaction.
- [ ] A query with a typo gets an AI fix shown as a diff, tested with OpenAI, Anthropic, Gemini, DeepSeek and Ollama.
- [ ] CI enforces every performance budget; the release workflow produces `.dmg`, `.msi`, NSIS `.exe`, `.deb`, `.rpm`, `.AppImage` and a valid PKGBUILD.
- [ ] Unit tests cover value formatting, the statement splitter and completion context; integration tests run against PostgreSQL and ClickHouse containers.

## Decisions

All open questions are settled; these answers override anything above that disagrees.

| Question | Decision |
| --- | --- |
| Client ID | Your session identity, sent as `application_name` on PostgreSQL (and as the client name on ClickHouse). Set on first launch. |
| Code signing | Ship unsigned while the app matures. A step-by-step walkthrough for Apple and Windows signing comes before the first public release. |
| Arch Linux | Follow the Arch norm: an AUR package, `rowboat-sql-bin`, that installs the official release build. Users run `yay -S rowboat-sql-bin` (or `paru`). CI checks the PKGBUILD with `makepkg` in an Arch container; publishing to the AUR needs one SSH-key secret, added later. |
| Default row limit | 1,000, switchable in one click from the results toolbar. |
| ClickHouse protocol | HTTP(S) with keep-alive, compression and binary `RowBinaryWithNamesAndTypes` rows. For page-sized results the query, not the protocol, is the bottleneck; native TCP is revisited only if benchmarks show a gap. |
| Telemetry | None. No usage stats and no crash reports. |
| Name | Rowboat (was DBPilot). Rowboat Labs already ships an unrelated, popular open-source desktop AI app called Rowboat, so package names that must be unique use `rowboat-sql` (e.g. the AUR package `rowboat-sql-bin`). |
