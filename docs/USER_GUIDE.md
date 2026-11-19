# Rowboat user guide

Rowboat is a SQL client for PostgreSQL, ClickHouse and SQLite with an AI
copilot. This guide covers what you can do with it today. Shortcuts are written
with `Ctrl`; on macOS use `⌘` instead. Every shortcut can be changed in
**Settings → Keyboard**.

## First launch

A short setup runs the first time:

1. **Client ID.** The name Rowboat uses for your sessions. It is sent as
   `application_name` on PostgreSQL and as the client name on ClickHouse, so
   whoever runs the database can see who is running what. This is the only
   required step.
2. **AI provider.** Choose a provider, paste its API key, load the model list,
   pick a model and press **Test**. Or choose **Skip — no AI**.
3. **Look and feel.** Theme, editor font size and default row limit.
4. **First connection.** Add your own, or open the sample database (a small
   SQLite shop with customers, products and orders).

You can run the setup again from **Settings → General**.

## Connections

Press `Ctrl+Shift+N` or the **+** in the sidebar. You can paste a connection
URL to fill in the form.

- **PostgreSQL** and **ClickHouse**: host, port, database, user and password,
  with TLS (from "disable" to "verify-full") and an optional SSH tunnel
  (password, key file or agent).
- **SQLite**: a file path.
- **Environment**: mark a connection as Prod to get a red accent and a
  confirmation before `DROP`, `TRUNCATE`, `ALTER`, and `UPDATE` or `DELETE`
  without `WHERE`.
- **Read-only**: Rowboat opens the connection so that it cannot write.
- **AI copilot**: turn AI off for a connection and nothing about it is ever
  sent to a provider.

Passwords and passphrases are stored in your operating system's keychain. If
the system has none (some Linux setups), Rowboat says so in the status bar and
keeps them in memory until you quit.

Rowboat reads a saved password only when you connect, and an API key only when
you send a request. On macOS the first read may ask for your login password;
choose **Always Allow** so it does not ask again.

## Writing and running SQL

| Action                                               | Shortcut            |
| ---------------------------------------------------- | ------------------- |
| Run the statement under the cursor, or the selection | `Ctrl+Enter`        |
| Run every statement in the tab                       | `Ctrl+Shift+Enter`  |
| Cancel                                               | `Ctrl+.`            |
| Show the plan (EXPLAIN)                              | `Ctrl+E`            |
| Format SQL                                           | `Ctrl+Shift+F`      |
| Open / save a `.sql` file                            | `Ctrl+O` / `Ctrl+S` |
| Query history                                        | `Ctrl+Y`            |
| New tab / close tab                                  | `Ctrl+T` / `Ctrl+W` |

Autocomplete knows your schema: tables after `FROM` and `JOIN`, the columns of
the tables in scope, aliases, CTEs, and ready-made `JOIN … ON` clauses from
foreign keys.

**Transactions.** Auto-commit is on by default and is set per connection from
the toolbar. With it off, statements run in a transaction until you press
**Commit** or **Rollback**.

## Results

- **Row limit.** A query fetches the first 1,000 rows by default. The limit is
  applied while fetching; your SQL is never rewritten. Change it for a tab from
  the results toolbar, or fetch the next page or everything.
- **Count rows** counts the full result without fetching it.
- **Sort** by clicking a column header, **filter** with the box in the toolbar,
  **resize** and **hide** columns.
- **Record** shows one row as a form; JSON values open formatted in the side
  panel.
- **Edit** a cell by double-clicking it when the result comes from one table
  with its primary key. Edits are staged: **Preview SQL** shows the exact
  `UPDATE` statements, and **Apply** runs them in one transaction.
- **Export** to CSV, TSV, JSON, NDJSON, SQL `INSERT` or Markdown. Exporting the
  full result streams it to the file without loading it into memory.

## The AI copilot

The copilot writes, fixes and explains SQL. It never runs anything: what it
writes goes into the editor, and you decide whether to run it.

- **Ask** (`Ctrl+K`): describe what you want. Or write a `-- comment` in the
  editor and press `Ctrl+K` on it. **Insert**, **Replace selection** or
  **Copy** the result.
- **Fix my query**: when a statement fails with a syntax or unknown-object
  error, a suggested fix appears under the error as a diff. **Accept** puts it
  in the editor; **Dismiss** hides it. Turn the automatic suggestion off in
  **Settings → AI** and a button asks for one instead.
- **Explain with AI** (`Ctrl+Shift+K`): a plain-English explanation of the
  selection or the statement under the cursor.

**What is sent.** Your question, the SQL involved, and a compact description of
your schema: table and column names, types, keys, indexes and comments. Result
rows are never sent. **View context** under any answer shows the exact request.
For PostgreSQL you can opt in, per connection, to also send a few common values
of low-cardinality columns (they help with `WHERE` filters); that is off by
default. Only syntax and unknown-object errors are sent for fixes; connection
and permission errors stay on your machine.

**Providers.** OpenAI, Anthropic, Google Gemini, DeepSeek, Mistral, xAI,
OpenRouter, Azure OpenAI, a local Ollama or LM Studio (no key needed), or any
OpenAI-compatible endpoint. API keys are stored in the OS keychain.

## Privacy

- No account, no telemetry, no crash reports.
- The app loads no remote content.
- The only network requests Rowboat makes are to your databases, to the AI
  provider you configured (when you use the copilot), and an update check that
  asks GitHub which releases exist and sends nothing else. Turn the update
  check off in **Settings → General**.
- Settings are a plain JSON file with no secrets in it; **Settings → Privacy &
  data** shows where it is and can export or import it.

## Where things are stored

**Settings → Privacy & data → Open data folder** opens it. It holds
`settings.json` (preferences), `rowboat.db` (connections without their
passwords, query history, open tabs, schema indexes) and, if you opened it,
`sample.db`.
