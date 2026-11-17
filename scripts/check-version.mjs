// The app version lives in four files. This fails when they disagree, and,
// given a tag (`node scripts/check-version.mjs v0.1.0`), when the tag does not
// match either.

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const read = (path) => readFileSync(join(root, path), 'utf8');

const versions = {
  'package.json': JSON.parse(read('package.json')).version,
  'src-tauri/tauri.conf.json': JSON.parse(read('src-tauri/tauri.conf.json')).version,
  'src-tauri/Cargo.toml': /^version\s*=\s*"([^"]+)"/m.exec(read('src-tauri/Cargo.toml'))?.[1],
  'packaging/arch/PKGBUILD': /^_upstreamver=(\S+)/m.exec(read('packaging/arch/PKGBUILD'))?.[1],
};

for (const [file, version] of Object.entries(versions)) console.log(`${version}\t${file}`);

const unique = new Set(Object.values(versions));
if (unique.size !== 1 || unique.has(undefined)) {
  console.error('FAIL: versions disagree.');
  process.exit(1);
}

const tag = process.argv[2];
if (tag && tag !== `v${[...unique][0]}`) {
  console.error(`FAIL: tag ${tag} does not match version ${[...unique][0]}.`);
  process.exit(1);
}
console.log('OK');
