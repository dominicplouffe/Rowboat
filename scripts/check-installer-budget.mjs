// Enforces the "Installer size < 20 MB" budget on whatever installers
// `tauri build` produced for this platform.
//
// AppImage is reported but exempt: it bundles its own copy of WebKitGTK and
// the libraries around it (~80 MB), which is the format, not our code. The
// .deb and .rpm, which use the system webview like every other platform, are
// the Linux installers the budget applies to.

import { existsSync, readdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const BUDGET_MB = Number(process.env.INSTALLER_BUDGET_MB ?? 20);
const INSTALLERS = /\.(dmg|msi|exe|deb|rpm|AppImage)$/;
const EXEMPT = /\.AppImage$/;

function* walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      // .app bundles and unpacked package trees are not installers.
      if (!/\.(app|AppDir)$/.test(entry.name) && !existsSync(join(path, 'DEBIAN')))
        yield* walk(path);
    } else if (INSTALLERS.test(entry.name)) {
      yield path;
    }
  }
}

// `--target universal-apple-darwin` etc. put bundles under target/<triple>/.
const targetDir = join(root, 'src-tauri/target');
const bundleDirs = [join(targetDir, 'release/bundle')];
if (existsSync(targetDir)) {
  for (const entry of readdirSync(targetDir)) {
    bundleDirs.push(join(targetDir, entry, 'release/bundle'));
  }
}

const found = bundleDirs.filter(existsSync).flatMap((dir) => [...walk(dir)]);
if (found.length === 0) {
  console.error('No installers found. Run `npm run tauri build` first.');
  process.exit(2);
}

let failed = false;
for (const path of found) {
  const mb = statSync(path).size / 1024 / 1024;
  const exempt = EXEMPT.test(path);
  const over = mb > BUDGET_MB && !exempt;
  failed ||= over;
  const status = exempt ? 'exempt' : over ? 'FAIL  ' : 'ok    ';
  console.log(`  ${status}  ${mb.toFixed(2).padStart(7)} MB  ${path.slice(root.length + 1)}`);
}

console.log(`\nInstaller budget: ${BUDGET_MB} MB`);
if (failed) {
  console.error('FAIL: an installer is over budget.');
  process.exit(1);
}
console.log('OK');
