// Enforces the "Initial JS bundle < 400 KB gzipped" budget.
//
// "Initial" means what the webview must load before the window is usable: the
// scripts and module preloads referenced by dist/index.html, plus everything
// they import statically. Lazily loaded chunks (CodeMirror, Settings) are
// reported but do not count.

import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join, posix } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';

const dist = join(dirname(fileURLToPath(import.meta.url)), '..', 'dist');
const BUDGET_KB = Number(process.env.BUNDLE_BUDGET_KB ?? 400);

const html = readFileSync(join(dist, 'index.html'), 'utf8');
const entries = [...html.matchAll(/<(?:script|link)[^>]+(?:src|href)="([^"]+\.js)"/g)].map((m) =>
  m[1].replace(/^\.?\//, ''),
);
if (entries.length === 0) {
  console.error('No scripts found in dist/index.html. Run `npm run build` first.');
  process.exit(2);
}

// Follow static imports only; `import(...)` marks a lazy boundary.
const initial = new Set();
const queue = [...entries];
while (queue.length) {
  const file = queue.pop();
  if (initial.has(file)) continue;
  initial.add(file);
  const code = readFileSync(join(dist, file), 'utf8');
  const statics = code.matchAll(
    /(?:^|[;\n}])\s*(?:import|export)\s*(?:[^"'()]*?\sfrom\s*)?["'](\.[^"']+\.js)["']/g,
  );
  for (const [, spec] of statics) queue.push(posix.join(posix.dirname(file), spec));
}

const gz = (file) => gzipSync(readFileSync(join(dist, file)), { level: 9 }).length;
const kb = (bytes) => `${(bytes / 1024).toFixed(1)} KB`;

const all = readdirSync(join(dist, 'assets'))
  .filter((f) => f.endsWith('.js'))
  .map((f) => `assets/${f}`);
let total = 0;
console.log('JS chunks (gzipped):');
for (const file of all.sort((a, b) => gz(b) - gz(a))) {
  const size = gz(file);
  const isInitial = initial.has(file);
  if (isInitial) total += size;
  console.log(`  ${isInitial ? 'initial' : 'lazy   '}  ${kb(size).padStart(10)}  ${file}`);
}

console.log(`\nInitial JS: ${kb(total)} gzipped (budget ${BUDGET_KB} KB)`);
if (total > BUDGET_KB * 1024) {
  console.error('FAIL: initial JS bundle is over budget.');
  process.exit(1);
}
console.log('OK');
