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

> **Status: v0.1 is feature-complete and not yet released.** Everything below
> works on Linux, where it was built and tested. It has not been built on macOS
> or Windows yet, ClickHouse has not been run against a real server, and the
> copilot has not been run against real AI providers. Where each acceptance
> criterion stands is in [docs/ACCEPTANCE.md](docs/ACCEPTANCE.md).

## What it does

- **Connects** to PostgreSQL, ClickHouse and SQLite, with TLS and SSH tunnels.
  Passwords live in the OS keychain.
- **Editor** with schema-aware autocomplete (aliases, CTEs, foreign-key joins),
  a formatter, EXPLAIN, history and `.sql` files.
- **Results** limited at fetch time (1,000 rows by default, one click to
  change), with paging, sorting, filtering, a JSON viewer, inline editing and
  streaming export to CSV, JSON, SQL and more.
- **AI copilot** that writes, fixes and explains SQL using your schema, with
  the provider you choose: OpenAI, Anthropic, Gemini, DeepSeek, Mistral, xAI,
  OpenRouter, Azure OpenAI, a local Ollama or LM Studio, or any
  OpenAI-compatible endpoint. It never runs SQL itself and never sends your
  rows.

The [user guide](docs/USER_GUIDE.md) covers all of it, including exactly what
the copilot sends and where your data is stored.

## Install

There is no release to download yet. Once there is, installers will be on the
[Releases page](https://github.com/dominicplouffe/Rowboat/releases):

| Platform                        | File                                                 |
| ------------------------------- | ---------------------------------------------------- |
| macOS (Apple Silicon and Intel) | `.dmg`                                               |
| Windows                         | `.msi` or `-setup.exe`                               |
| Debian / Ubuntu                 | `.deb`                                               |
| Fedora / RHEL                   | `.rpm`                                               |
| Any Linux                       | `.AppImage`                                          |
| Arch Linux                      | `yay -S rowboat-sql-bin` (once published to the AUR) |

Until then, build from source (below).

## Build from source

There is no installer yet, so this is how to get Rowboat running. You need three things: your
platform's build tools, [Rust](https://rustup.rs) and [Node.js](https://nodejs.org) 22 or newer.

### 1. Install the build tools

**macOS.** Install the Xcode Command Line Tools (the full Xcode app is not needed). A dialog
opens; choose **Install** and wait for it to finish:

```bash
xcode-select --install
```

**Windows.** Install the
[Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) and
select the "Desktop development with C++" workload. WebView2 is already part of Windows 10
(version 1803 and later) and Windows 11.

**Debian / Ubuntu.** Other distributions are covered in the
[Tauri prerequisites](https://tauri.app/start/prerequisites/).

```bash
sudo apt-get update
```

```bash
sudo apt-get install -y build-essential pkg-config file patchelf rpm libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libxdo-dev
```

### 2. Install Rust

macOS and Linux (accept the default installation when asked):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

On Windows, download and run `rustup-init.exe` from [rustup.rs](https://rustup.rs).

### 3. Install Node.js 22 or newer

Download the installer from [nodejs.org](https://nodejs.org), or on macOS with
[Homebrew](https://brew.sh):

```bash
brew install node
```

### 4. Check the tools

Open a **new** terminal so the freshly installed tools are found, then check each one answers:

```bash
rustc --version
```

```bash
node --version
```

### 5. Get the code and run it

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

The first run compiles the Rust core, which takes several minutes; later runs take seconds. The
Rowboat window opens when it is done, starting with a short setup. Choose **Open the sample
database** on its last step to have something to query straight away.

### Build an installer

This builds the app and an installer for your platform (a `.dmg` on macOS) into
`src-tauri/target/release/bundle/`:

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
| Installer size budget                   | `npm run budget:installer`                          |
| Regenerate icons from the logo          | `npm run icons`                                     |

Architecture notes and project rules live in [CLAUDE.md](CLAUDE.md). Other docs:
[product spec](docs/SPEC.md), [user guide](docs/USER_GUIDE.md),
[acceptance status](docs/ACCEPTANCE.md), [releasing](docs/RELEASING.md),
[code signing](docs/SIGNING.md), [changelog](CHANGELOG.md).

## Privacy

No account, no telemetry, no crash reports, and the app loads no remote content. Rowboat talks to
your databases, to the AI provider you set up (only when you use the copilot), and to GitHub for
an update check that sends nothing about you and can be turned off.

## License

[MIT](LICENSE) © Dominic Plouffe

Rowboat is not affiliated with Rowboat Labs or its Rowboat application. Packages whose names must
be unique use `rowboat-sql` (for example the AUR package `rowboat-sql-bin`).
