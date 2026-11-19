# Rowboat

A fast, lightweight SQL client (PostgreSQL, ClickHouse, SQLite) with an AI copilot.
Tauri 2 + Rust core + Svelte 5 + CodeMirror 6. MIT, © Dominic Plouffe.

The product spec is `docs/SPEC.md`. Read it before changing behavior. Its
"Decisions" table overrides anything earlier in the spec.

## Status

Built milestone by milestone (spec → "Milestones"), one branch and one PR each.

- [x] **M0 Foundation** — app scaffold, settings store, brand assets, CI + release workflows
  - CI is active and green on macOS, Windows and Linux. Every green push to main
    replaces the rolling `latest-build` pre-release with that commit's unsigned
    installers.
- [x] **M1 Connections** — three drivers, TLS, SSH tunnels, keychain secrets, run and cancel,
      schema navigator, Prod guard
  - `tests/clickhouse.rs` runs against a real ClickHouse in CI's integration job.
- [x] **M2 Editor** — schema-aware autocomplete, paging, grid sort/filter/resize/hide, inline
      editing, JSON viewer, record view, export, history, session restore, formatter, EXPLAIN
  - Cold start measures 0.93–0.99 s on the GPU-less build VM: inside the 1 s budget with no
    headroom. Anything added to the startup path must be lazy.
- [x] **M3 AI copilot** — three provider adapters, schema index, Ask / Fix / Explain, AI settings,
      first-launch wizard, sample database
  - Anthropic works for real (Ask, Fix and Explain, on macOS). The other adapters have only
    been tested against a local fake server that speaks each documented wire format.
- [x] **M4 Release** — update notice, grid budget test, user guide, signing and release docs,
      acceptance status
  - **v0.1 is not tagged.** The release gate is "every CI budget green". What is left is in `docs/RELEASING.md`; where each acceptance criterion stands is in
    `docs/ACCEPTANCE.md`. Keep both current.

## Commands

```bash
npm install                 # once
npm run tauri dev           # run the app with hot reload
npm run dev                 # UI only, in a browser, against an in-memory fake core

npm test                    # Vitest (frontend unit tests)
npm run check               # svelte-check (types)
npm run lint                # ESLint + Prettier check
npm run format              # Prettier write
cd src-tauri && cargo test  # Rust unit tests (needs dist/: run `npm run build` once)
cd src-tauri && cargo fmt --check && cargo clippy --all-targets --features integration -- -D warnings

docker compose up -d --wait # PostgreSQL + ClickHouse for integration tests
eval "$(scripts/dev-sshd.sh)" # a throwaway sshd for the tunnel tests (scripts/dev-sshd.sh stop)
cd src-tauri && cargo test --features integration   # tests/{postgres,clickhouse,ssh,services}.rs

npm run tauri build         # installers for this OS, in src-tauri/target/release/bundle/
npm run budget:bundle       # initial JS < 400 KB gzipped   (after `npm run build`)
npm run budget:installer    # installers < 20 MB            (after `npm run tauri build`)
npm run budget:startup      # cold start < 1 s; idle memory < 150 MB on Linux
                            # (Linux without a display: `xvfb-run -a npm run budget:startup`;
                            #  without a GPU also set WEBKIT_DISABLE_DMABUF_RENDERER=1, or software
                            #  OpenGL adds >100 MB; `-- --first-launch` measures with the wizard)

npm run icons               # regenerate every icon/brand asset from assets/brand/*.svg
node scripts/check-version.mjs   # the 4 files that carry the version agree
```

Linux build dependencies are listed in `.github/actions/setup/action.yml`.

## Architecture

```
src/                     Svelte 5 UI (runes mode). Renders only.
  lib/ipc.ts             The single doorway to the Rust core (typed `invoke` wrappers).
  lib/devBackend.ts      In-memory fake core for browser dev and tests; never in prod bundles.
  lib/settings.svelte.ts Reactive mirror of the core's settings.
  lib/settingsSchema.ts  Declarative description of the Settings window (fields, search).
  lib/shortcuts.ts, actions.ts   Keyboard chords and the action registry (rebindable).
  lib/tabs.svelte.ts     Open query tabs; each has its own row limit, connection and results.
  lib/connections.svelte.ts  Saved connections and their open state.
  lib/runner.ts          Which statements run, in what order; confirmation and error handling.
  lib/schema.svelte.ts   The navigator's lazily loaded tree; SQL name/template helpers.
  lib/dialogs.svelte.ts  Confirm prompts and the connection dialog.
  lib/complete.ts        Completion engine: context, aliases/CTEs, FK joins, fuzzy ranking. Pure.
  lib/schemaCache.svelte.ts  In-memory schema index per connection (what completion reads).
  lib/cells.ts           Value display (time zone, number format, binary), sorting, filtering.
  lib/editing.ts         Staged inline edits → row edits for the core.
  lib/session.ts         Saves and restores open tabs and unsaved text.
  lib/format.ts          SQL formatter (sql-formatter, loaded on first use).
  lib/ai.svelte.ts       Copilot UI state: Ask panel, suggested fixes, which provider is ready.
  lib/aiText.ts, diff.ts Comment-as-prompt, streamed-answer parsing; line diff for fixes. Pure.
  lib/providers.ts       The provider catalogue the forms show (labels, default URLs, key rules).
  editor/sqlEditor.ts    CodeMirror 6. Lazy chunk, loaded after first paint.
  components/            Sidebar, SchemaNodes, TabBar, Workspace, Editor, ResultsPane, Grid,
                         SidePanel (JSON viewer + record view), JsonNode, PlanTree, Menu,
                         RowLimitMenu, StatusBar, SettingsDialog, ConnectionDialog,
                         ConfirmDialog, HistoryDialog, AskPanel, FixSuggestion, AiContext,
                         AiProviders, ProviderForm, Onboarding (all AI/wizard UI is lazy).
src-tauri/
  src/lib.rs             Tauri setup and IPC commands.
  src/manager.rs         Connections + secrets + tunnels + open sessions; what the commands call.
  src/db/mod.rs          `Session` trait, result/error types shared by the drivers.
  src/db/postgres.rs     tokio-postgres; an actor owns the connection (transaction state machine).
  src/db/pg_types.rs     Renders PostgreSQL binary values as text.
  src/db/clickhouse.rs   HTTP(S) via reqwest; streams and cuts at the row limit.
  src/db/ch_rowbinary.rs Incremental RowBinaryWithNamesAndTypes decoder.
  src/db/sqlite.rs       rusqlite on its own thread.
  src/statements.rs      Statement splitter, transaction-control and Prod-guard detection (sqlparser tokens).
  src/connections.rs     Connection config + the app's own SQLite file (rowboat.db):
                         connections, query history, saved session.
  src/export.rs          CSV / TSV / JSON / NDJSON / SQL INSERT / Markdown writers (RowSink).
  src/edits.rs           Staged edits → UPDATE statements (what Preview SQL shows).
  src/ai/mod.rs          Provider kinds, HTTP plumbing, errors; dispatch to the three adapters.
  src/ai/openai_compat.rs, anthropic.rs, gemini.rs   One adapter per API family (SSE streaming).
  src/ai/sse.rs          Incremental Server-Sent Events parser.
  src/ai/schema_index.rs Compact schema for prompts: build, fingerprint, rank, token budget.
  src/ai/copilot.rs      Ask / Fix / Explain: prompt, stream, parse check + one retry, cancel.
  src/ai/tests.rs        Adapters and copilot against a local fake provider.
  src/sample.rs          Generates the sample SQLite database the wizard can open.
  src/updates.rs         Update check: asks GitHub for releases, compares versions. Notify only.
  src/secrets.rs         OS keychain (keyring); in-memory for the session when there is none.
  src/ssh.rs             SSH tunnels (russh) and known_hosts checking.
  src/tls.rs             rustls setup for the sslmode levels.
  src/settings.rs        Settings types, validation, atomic persistence. No secrets, by design.
  src/error.rs           Error type that crosses IPC as a string.
  capabilities/          What the webview may call. Keep it minimal.
  tests/services.rs      Integration harness smoke test (feature `integration`).
  icons/, installer/     Generated by `npm run icons`. Do not edit by hand.
assets/brand/            Logo sources: icon.svg (master), logo.svg (mark), glyph.svg (16–32 px).
packaging/arch/          PKGBUILD for the AUR package rowboat-sql-bin + validate.sh.
docs/                    SPEC.md (product spec), USER_GUIDE.md, ACCEPTANCE.md, RELEASING.md, SIGNING.md.
scripts/                 Budget checks, version check, brand generator.
.github/                 workflows: ci.yml (every push/PR), release.yml (tag v*); actions/setup.
```

How a setting flows: UI control → `settings.set()` → `ipc.settingsSet` → Rust
`SettingsStore::set` normalizes/clamps and writes `settings.json` atomically →
the normalized copy comes back and the UI adopts it. `types.ts`,
`devBackend.ts` and `settings.rs` describe the same shape: change them together
(`settingsSchema.test.ts` fails if a setting has no field in the UI).
`src/lib/settings.defaults.json` pins the shape for both sides: a Rust test and
`settingsContract.test.ts` each compare against it. After an intended change,
regenerate it with `UPDATE_FIXTURES=1 cargo test default_settings_match`. A
setting nothing reads yet carries `arrives: 'M1'…'M4'` in the schema and is
shown disabled; remove the marker when the feature lands.

How a query flows: Run → `lib/runner.ts` asks the core to split the text
(`sql_split`) and sends statements one at a time (`query_run`) exactly as
written → `Manager::run` applies the Prod guard and the timeout → the driver
fetches at most the tab's row limit. Errors come back as a `DbError` with a
`kind`; only `syntax` and `schema` errors will be sent to the AI (M3).

Sessions: statements on one connection run one at a time (`Manager` holds a
per-connection turn), so a queued statement can be cancelled without touching
the running one. Auto-commit is per open connection and starts from the
Settings default. The PostgreSQL navigator uses its own second connection, so
catalog queries never run inside (or wait behind) the user's transaction.

Results after the first page: "Fetch next page" and "Fetch all" run the
statement again with a larger fetch window (no cursor or transaction is held
open between pages); "Fetch all" stops at the result memory cap unless the user
lifts it. "Count rows" wraps the query in `SELECT count(*) FROM (…)` as a
separate statement. Export of a full result streams rows from the driver
straight to the file (`Session::export` + `RowSink`). Result rows are kept out
of Svelte's deep reactivity (`unproxied` in runner.ts): replace the result
object to update the grid, do not mutate rows.

Inline editing: a result is editable when every source column comes from one
table and its whole primary key is present (`QueryResult::editable`). Edits
are staged in the UI, turned into one UPDATE per row by `edits.rs`, and applied
in a single transaction (a savepoint when the user already has one open).

Driver rules: values leave the driver already rendered as text; the row limit
is enforced while fetching (portal with a row count / `max_result_rows` and a
client-side cut / stop stepping); cancel must return within a second.

How an AI request flows: the UI calls `ai_run` with a task (ask / fix /
explain) and a Tauri channel → `Copilot::run` picks the provider in use, reads
its key from the keychain, builds the prompt from the connection's schema
index, and streams the answer back over the channel → generated SQL is parsed
with sqlparser (one retry with the parser error if it fails) and checked with
`destructive_reason` → the UI shows it. Accept / Insert only edit the editor
text. There is no code path from the copilot to `query_run`: keep it that way.
The first channel event is the exact payload sent ("View context").

Schema index: built from `Session::snapshot` + `Session::details` on connect
(only once a provider exists), stored in rowboat.db under `schema-index/<id>`
with a fingerprint; a refresh rewrites it only when the fingerprint changed.
Sample values are per connection, off by default, PostgreSQL only (from
`pg_stats`, no table scan). `ConnectionConfig::ai_enabled = false` means
nothing about that connection is sent. Fix requests are refused in Rust unless
the error kind is `syntax` or `schema`.

Provider settings (`settings.ai.providers`) hold no secret; the key is saved
with `ai_key_set` under `ai/<provider id>/key` and never returned. Model lists
come from each provider's models endpoint; no model name is hard-coded.

## Non-negotiables

- **Speed and memory are the product.** Budgets from the spec's "Performance
  budget" table are enforced in CI. Lazy-load anything not needed at startup
  (dynamic `import()`); the initial bundle is Svelte + the shell only. No UI kit.
- **Stack is fixed:** Tauri 2, Rust core, Svelte 5, CodeMirror 6, and the crates
  named in the spec's tech-stack table. Ask before swapping any of it.
- **All database, AI and secret work happens in Rust.** API keys and passwords
  never reach the webview: the UI sends a secret once on entry and never reads
  it back.
- **Secrets only in the OS keychain** (`keyring` crate). Never in
  `settings.json`, logs, error messages or exports. `Settings` must not grow a
  field that can hold a secret.
- **Row limit (default 1,000) is applied at fetch time, never by rewriting the
  user's SQL.** PostgreSQL: server-side cursor. ClickHouse: `max_result_rows` +
  `result_overflow_mode=break`. SQLite: stop stepping. One click to change from
  the results toolbar.
- **Client ID** (set at first launch, `general.clientId`) is sent as
  `application_name` on PostgreSQL and as the client name on ClickHouse.
- **ClickHouse goes over HTTP(S)** with keep-alive, compression and
  `RowBinaryWithNamesAndTypes`.
- **AI never executes SQL.** Fixes are suggested automatically on syntax/schema
  errors, shown as a diff with Accept/Dismiss. Every provider in the spec's
  table is supported, DeepSeek included.
- **No telemetry and no crash reporting of any kind.** The webview loads no
  remote content (strict CSP). The only requests the core makes on its own are
  the update check (the release list from GitHub, nothing about the user, off
  when the setting is off). Do not add another.
- **MIT license, copyright "Dominic Plouffe".**
- **Use current stable versions** of crates and npm packages; check the docs
  rather than relying on memory.
- **Never push to main.** One branch and one PR per milestone.

## Conventions

- Svelte 5 runes only (`$state`, `$derived`, `$effect`, `$props`); shared state
  lives in `*.svelte.ts` classes.
- Styling is plain CSS with the tokens in `src/app.css`; both themes must work.
  Component styles are scoped.
- Every action gets a shortcut via `lib/actions.ts`; nothing requires one.
- UI that is not functional yet is disabled and says which milestone brings it.
  Do not fake behavior.
- New IPC commands: add to `lib.rs` `generate_handler!`, wrap in `lib/ipc.ts`,
  fake in `lib/devBackend.ts`.
- Tests assert behavior (what a user or caller observes), not implementation.
- Workflows must pass with no signing secrets set.
