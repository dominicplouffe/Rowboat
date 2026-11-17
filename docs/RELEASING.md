# Releasing Rowboat

## Before the first release

These are open as of the v0.1 work. None can be done from the development
session that wrote the code.

1. **Activate the workflows.** They are complete but sit in
   `.github/workflows.pending/` because the session that pushed them was not
   allowed to create workflow files. From a clone where you can push workflows:

   ```bash
   git rm .github/workflows.pending/README.md
   ```

   ```bash
   git mv .github/workflows.pending .github/workflows
   ```

   ```bash
   git commit -m "Activate CI and release workflows"
   ```

   ```bash
   git push
   ```

2. **Get CI green.** It has never run. The Linux jobs match what was run by
   hand during development. The macOS and Windows jobs have never built, so
   expect first-run fixes there (most likely in packaging, or in tests that
   assume a Unix path). The ClickHouse integration tests have also never run
   against a real server.
3. **Check the startup numbers on real hardware.** CI holds hosted runners to a
   looser cold-start number (2.5 s, 4 s on Windows) because they are slow VMs
   without a GPU. The spec's 1 s target is for a 2021+ laptop:

   ```bash
   npm run tauri build
   ```

   ```bash
   npm run budget:startup
   ```

4. **Try the copilot with real providers.** The acceptance criteria name
   OpenAI, Anthropic, Gemini, DeepSeek and Ollama. Development could only test
   against a fake server speaking each documented format.
5. **Walk through [ACCEPTANCE.md](ACCEPTANCE.md)** on macOS, Windows and
   Ubuntu.

## Cutting a release

1. Set the version in the four places that carry it (`package.json`,
   `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`,
   `packaging/arch/PKGBUILD`) and check they agree:

   ```bash
   node scripts/check-version.mjs
   ```

2. Update [CHANGELOG.md](../CHANGELOG.md).
3. Merge to `main` with CI green.
4. Tag and push. The tag must match the version (`v0.1.0` for `0.1.0`); a tag
   with a hyphen (`v0.2.0-beta.1`) becomes a pre-release:

   ```bash
   git tag v0.1.0
   ```

   ```bash
   git push origin v0.1.0
   ```

5. The Release workflow builds every installer and attaches them to a **draft**
   GitHub Release. Nothing is public yet. Download each installer, install it,
   and check it starts.
6. Publish the draft. Installed copies with update checks on will show
   "Rowboat x.y.z is available" the next time they start.
7. For Arch, push the PKGBUILD from the workflow's `rowboat-sql-bin-aur`
   artifact to the AUR (see [SIGNING.md](SIGNING.md)).

## What the release contains

| Platform                      | Files                                                       |
| ----------------------------- | ----------------------------------------------------------- |
| macOS (Apple Silicon + Intel) | `.dmg`                                                      |
| Windows x64                   | `.msi`, NSIS `-setup.exe`                                   |
| Linux x64                     | `.deb`, `.rpm`, `.AppImage`                                 |
| Arch Linux                    | `rowboat-sql-bin` PKGBUILD (installs the `.deb`'s contents) |

Installers are unsigned until [SIGNING.md](SIGNING.md) has been followed.
