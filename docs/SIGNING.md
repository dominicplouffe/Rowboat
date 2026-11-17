# Code signing walkthrough

Rowboat ships unsigned while it matures (see the Decisions table in
[SPEC.md](SPEC.md)). Unsigned installers work, but macOS Gatekeeper and Windows
SmartScreen warn people who download them. This is the step-by-step path to
signed builds, to do before the first public release.

The release workflow already reads the macOS secrets below when they exist; no
workflow change is needed for macOS. Windows signing and the updater need the
small changes described in their sections.

None of this has been exercised yet: no certificate exists for the project.
Treat the commands as the documented procedure, and expect to adjust details
the first time through.

## macOS: Developer ID and notarization

You need a paid Apple Developer Program membership.

1. **Create the certificate.** In Xcode: Settings → Accounts → your team →
   Manage Certificates → **+** → _Developer ID Application_. (Or create it in
   the Apple Developer portal from a certificate signing request.)
2. **Export it.** In Keychain Access, find _Developer ID Application: Your Name
   (TEAMID)_, expand it, select the certificate and its private key, and export
   as a `.p12` file with a strong password.
3. **Encode it** for a GitHub secret:

   ```bash
   base64 -i DeveloperID.p12 | pbcopy
   ```

4. **Create an app-specific password** for notarization at
   <https://account.apple.com> → Sign-In and Security → App-Specific Passwords.
5. **Add the repository secrets** (GitHub → Settings → Secrets and variables →
   Actions):

   | Secret                       | Value                                          |
   | ---------------------------- | ---------------------------------------------- |
   | `APPLE_CERTIFICATE`          | The base64 text from step 3                    |
   | `APPLE_CERTIFICATE_PASSWORD` | The `.p12` password from step 2                |
   | `APPLE_SIGNING_IDENTITY`     | `Developer ID Application: Your Name (TEAMID)` |
   | `APPLE_ID`                   | Your Apple ID email                            |
   | `APPLE_PASSWORD`             | The app-specific password from step 4          |
   | `APPLE_TEAM_ID`              | The 10-character team ID                       |

6. **Remove the ad-hoc identity.** In `src-tauri/tauri.conf.json`, delete
   `"signingIdentity": "-"` under `bundle.macOS` so the identity from the
   environment is used. (With the line present and no secrets, builds get an
   ad-hoc signature, which is what makes the unsigned app start on Apple
   Silicon.) Keeping unsigned CI builds working after this change means setting
   `APPLE_SIGNING_IDENTITY=-` in `ci.yml` for the macOS build.
7. **Tag a pre-release** (for example `v0.1.1-rc.1`) and check the result on a
   Mac that has never run Rowboat:

   ```bash
   spctl --assess --type open --context context:primary-signature -v Rowboat.dmg
   ```

   ```bash
   xcrun stapler validate /Applications/Rowboat.app
   ```

## Windows: Authenticode

Two routes. Azure Trusted Signing is cheaper and needs no hardware token; an
OV/EV certificate from a certificate authority also works.

### Azure Trusted Signing

1. Create a Trusted Signing account and a certificate profile in the Azure
   portal, and complete identity validation.
2. Create an app registration with the _Trusted Signing Certificate Profile
   Signer_ role, and note its tenant ID, client ID and a client secret.
3. Add the secrets `AZURE_TENANT_ID`, `AZURE_CLIENT_ID` and
   `AZURE_CLIENT_SECRET` to the repository.
4. In `src-tauri/tauri.conf.json` set `bundle.windows.signCommand` to a
   `trusted-signing-cli` invocation for your endpoint, account and profile, and
   install that tool in the Windows job of `release.yml` before the build.
   Tauri's Windows signing guide has the current command line.

### A certificate file

1. Buy a code-signing certificate and export it as a `.pfx`.
2. Add it base64-encoded as the secret `WINDOWS_CERTIFICATE`, with
   `WINDOWS_CERTIFICATE_PASSWORD`.
3. In the Windows job of `release.yml`, import it into the certificate store
   before the build, and set `bundle.windows.certificateThumbprint`,
   `digestAlgorithm` (`sha256`) and `timestampUrl` in `tauri.conf.json`.

Either way, verify on a clean Windows machine:

```powershell
Get-AuthenticodeSignature .\Rowboat_0.1.0_x64-setup.exe
```

SmartScreen reputation builds with downloads; a new certificate may still show
a warning for a while (EV certificates and Trusted Signing less so).

## Updater signing (for installing updates automatically)

Today Rowboat only notifies about a new version and opens the download page.
Installing updates by itself needs update artifacts signed with a key that the
app trusts. The private key must be yours alone, which is why it has not been
generated for you.

1. Generate a key pair on your own machine:

   ```bash
   npm run tauri signer generate -- -w ~/.tauri/rowboat.key
   ```

2. Add the private key file's content as the secret
   `TAURI_SIGNING_PRIVATE_KEY`, and its password as
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Keep a backup: losing the key means
   installed copies can never be updated automatically again.
3. Add `tauri-plugin-updater` to the app, put the public key and the endpoint
   `https://github.com/dominicplouffe/Rowboat/releases/latest/download/latest.json`
   in `tauri.conf.json` (`plugins.updater`), and set
   `bundle.createUpdaterArtifacts` to `true`.
4. In `release.yml`, pass the two secrets to the `tauri-action` step and set
   `uploadUpdaterJson: true`.
5. Make "Install automatically" in **Settings → General** download and install
   through the plugin, and keep "Notify me" as it is.

## Arch Linux (AUR)

`packaging/arch/PKGBUILD` is validated with `makepkg` in CI. Publishing needs
an AUR account and one SSH key:

1. Create the `rowboat-sql-bin` package repository on the AUR with your
   account's SSH key.
2. Add the private key as the secret `AUR_SSH_PRIVATE_KEY`.
3. After a release is published (the PKGBUILD downloads the `.deb` from it),
   push the `PKGBUILD` and `.SRCINFO` from the workflow's `rowboat-sql-bin-aur`
   artifact to the AUR repository.
