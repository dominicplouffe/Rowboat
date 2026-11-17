<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/logo-wordmark-dark.svg">
    <img src="assets/brand/logo-wordmark-light.svg" alt="Rowboat" height="72">
  </picture>
</p>

<p align="center">A fast, lightweight SQL client with an AI copilot that knows your schema.</p>

---

Rowboat is a SQL client for macOS, Windows and Linux. v0.1 targets PostgreSQL,
ClickHouse and SQLite, with a first-class editor and data grid and nothing that
slows startup. The name is one word: **Row**boat, because every query brings
back rows.

- **Fast first.** Cold start under 1 s, idle memory under 150 MB, installer under 20 MB.
- **Keyboard-first, mouse-friendly.** Every action has a shortcut; nothing requires one.
- **AI is a copilot, not an autopilot.** AI-generated SQL never runs until you press Run.
- **Safe by default.** Secrets live in the OS keychain. No account, no telemetry, no crash reports.
- **Bring your own model.** Any major LLM provider, or a local model with no key at all.

> **Status: early development.** Milestone M0 (foundation) is in place: the app
> shell, settings, themes, the SQL editor, and build pipelines for every
> platform. Database connections arrive next. See the full plan in
> [docs/SPEC.md](docs/SPEC.md).

## Build from source

You need [Node.js](https://nodejs.org) 22+, [Rust](https://rustup.rs) (stable), and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS (on macOS: Xcode
Command Line Tools).

```bash
git clone https://github.com/dominicplouffe/Rowboat.git
```

```bash
cd Rowboat
```

```bash
npm install
```

```bash
npm run tauri dev
```

To build an installer for your platform (in `src-tauri/target/release/bundle/`):

```bash
npm run tauri build
```

Builds are unsigned for now, so macOS Gatekeeper and Windows SmartScreen will warn about
downloaded installers. On macOS, right-click the app and choose **Open** the first time, or clear
the quarantine flag:

```bash
xattr -dr com.apple.quarantine /Applications/Rowboat.app
```

## Development

| Task                                    | Command                                             |
| --------------------------------------- | --------------------------------------------------- |
| Frontend tests                          | `npm test`                                          |
| Rust tests                              | `cd src-tauri && cargo test`                        |
| Types and lint                          | `npm run check && npm run lint`                     |
| PostgreSQL + ClickHouse for integration | `docker compose up -d --wait`                       |
| Integration tests                       | `cd src-tauri && cargo test --features integration` |
| Performance budgets                     | `npm run budget:bundle`, `npm run budget:startup`   |
| Regenerate icons from the logo          | `npm run icons`                                     |

Architecture notes and project rules live in [CLAUDE.md](CLAUDE.md).

## License

[MIT](LICENSE) © Dominic Plouffe

Rowboat is not affiliated with Rowboat Labs or its Rowboat application. Packages whose names must
be unique use `rowboat-sql` (for example the AUR package `rowboat-sql-bin`).
