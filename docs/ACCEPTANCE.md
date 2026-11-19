# v0.1 acceptance status

The spec says v0.1 is done when every criterion holds on macOS, Windows and
Ubuntu. This is where each one stands. "Verified" means it was run and seen to
work during development, which happened on Linux (Ubuntu 24.04, PostgreSQL 16,
no GPU). CI now runs on every push: unit tests and installer builds with their
budgets on macOS, Windows and Linux, and integration tests on Linux against
PostgreSQL 18, ClickHouse 26.8 and an SSH server. The macOS build has also been
measured on real hardware (below). No criterion has been walked through by hand
in the app on macOS or Windows yet.

| #   | Criterion                                                                                                                                            | Linux                             | What backs it                                                                                                                                                                        | Still open                                                                                                    |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------- |
| 1   | First launch sets the Client ID; it shows as `application_name`; AI, theme and row-limit steps can be skipped                                        | Verified                          | Wizard driven in the real app; integration test reads `pg_stat_activity`                                                                                                             | macOS, Windows                                                                                                |
| 2   | Connects to PostgreSQL 13–18, current ClickHouse LTS and SQLite 3, with TLS and SSH tunnel; secrets only in the OS keychain                          | Partly                            | PostgreSQL 16 with TLS (all `sslmode` levels) and SSH tunnels (password, key, agent); PostgreSQL 18 and ClickHouse 26.8 in CI; SQLite; keychain code path with an in-memory stand-in | PostgreSQL 13–15, 17; ClickHouse TLS and SSH tunnel; a real OS keychain                                       |
| 3   | Runs statement, selection and script; cancels a 60-second query within 1 s on each database                                                          | Verified                          | PostgreSQL, SQLite and ClickHouse cancel tests (ClickHouse against a real server in CI)                                                                                              | Cancel driven by hand in the app                                                                              |
| 4   | `SELECT *` on a 10M-row table shows the first 1,000 rows without fetching the rest; memory under the cap                                             | Verified on PostgreSQL            | Real app against a 10M-row `generate_series`; row-limit and memory-cap tests                                                                                                         | ClickHouse by hand (CI covers its row limit on an endless table); a real 10M-row table                        |
| 5   | Autocomplete resolves aliases, CTEs and FK joins at < 30 ms p95 on a 500-table schema                                                                | Verified                          | Unit tests, including the timing on 500 tables                                                                                                                                       | Timing on other hardware                                                                                      |
| 6   | JSON cells open formatted; a 1M-row export streams to CSV under the memory cap                                                                       | Partly                            | JSON viewer in the real app; export streams row by row (tested with thousands of rows)                                                                                               | A 1M-row export measured for memory                                                                           |
| 7   | Inline edits preview the exact UPDATE and apply in one transaction                                                                                   | Verified on PostgreSQL and SQLite | Unit and integration tests; driven in the UI                                                                                                                                         | ClickHouse does not support it (no primary-key updates): by design                                            |
| 8   | A typo gets an AI fix shown as a diff, tested with OpenAI, Anthropic, Gemini, DeepSeek and Ollama                                                    | **Partly**                        | Anthropic (`claude-haiku-5-5`): Ask, Fix and Explain work in the real app on macOS; every adapter is also tested against a fake server speaking its documented format                | OpenAI, Gemini, DeepSeek and Ollama have never been called for real. Needs a key for each, and a local Ollama |
| 9   | CI enforces every budget; the release workflow produces `.dmg`, `.msi`, NSIS `.exe`, `.deb`, `.rpm`, `.AppImage` and a valid PKGBUILD                | Met in CI                         | CI builds `.dmg`, `.msi`, NSIS `.exe`, `.deb`, `.rpm` and `.AppImage`, runs every budget, and runs the PKGBUILD through `makepkg`                                                    | `release.yml` has never run (no tag pushed yet)                                                               |
| 10  | Unit tests cover value formatting, the statement splitter and completion context; integration tests run against PostgreSQL and ClickHouse containers | Verified                          | 182 Rust unit tests, 135 frontend tests; PostgreSQL, ClickHouse and SSH integration tests run in CI                                                                                  | None                                                                                                          |

## Performance budgets

Measured locally on the Linux build machine (a VM without a GPU).

| Metric                        | Target                 | Measured                                                                         | Enforced by                                                              |
| ----------------------------- | ---------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| Cold start to usable window   | < 1 s                  | 0.93 s (everyday launch), 0.92 s (first launch)                                  | `npm run budget:startup`                                                 |
| Idle memory, app + webview    | < 150 MB               | 77 MB private                                                                    | `npm run budget:startup` (Linux)                                         |
| Installer size                | < 20 MB                | 5.8 MB (`.deb`, `.rpm`); the 81 MB AppImage is exempt: it carries its own WebKit | `npm run budget:installer`                                               |
| Initial JS bundle             | < 400 KB gzipped       | 55 KB                                                                            | `npm run budget:bundle`                                                  |
| Autocomplete latency          | < 30 ms p95            | under budget on 500 tables                                                       | Vitest (`complete.test.ts`)                                              |
| Grid scrolling with 100k rows | 60 fps                 | not measured as a frame rate                                                     | Vitest (`Grid.test.ts`) checks that only the rows in view are in the DOM |
| Result memory                 | capped, default 256 MB | cap enforced while fetching                                                      | Rust tests                                                               |

Cold start has little headroom: about 70 ms on this machine. The frame rate of
the grid has not been measured on a real display.

### macOS on real hardware

Measured on an M1 Pro (macOS 26.4) with the release build.

| Metric                      | Target   | Measured                                                                   |
| --------------------------- | -------- | -------------------------------------------------------------------------- |
| Cold start to usable window | < 1 s    | 0.48 s median of 3 runs (everyday launch); first launch the same           |
| Idle memory, app + webview  | < 150 MB | about 87 MB for the app and its WebKit processes, by hand with `footprint` |
| Installer size              | < 20 MB  | 5.0 MB (`.dmg`, Apple silicon only; CI builds the universal one)           |

`npm run budget:startup` measures idle memory on Linux only, which is why the
macOS figure was taken by hand.
