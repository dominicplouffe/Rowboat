// Enforces the cold-start budget (and, on Linux, the idle-memory budget).
//
//   node scripts/check-startup-budget.mjs [path/to/rowboat]
//
// The app is launched several times with ROWBOAT_BENCH_OUT set. In that mode
// it writes the time from process start to the first painted, usable window to
// that file and exits. The clock starts when this script spawns the process
// and stops when the SQL editor is loaded and painted, so it covers process
// creation, loading the webview, and the lazily loaded editor. It is a local measurement: nothing is sent anywhere.
//
// Budgets (override with env vars):
//   STARTUP_BUDGET_MS      default 1000 — the spec's target on a 2021+ laptop.
//                          CI sets a looser number because hosted runners are
//                          slow, shared VMs without a GPU; see ci.yml.
//   IDLE_MEMORY_BUDGET_MB  default 150 (200 on Windows). Measured on Linux
//                          only for now, over the app and its webview processes.
//
// What "idle memory" means here: private memory (anonymous + shared-memory
// pages, proportionally attributed), i.e. what the app itself allocates and
// what desktop system monitors show as "Memory". The total including the
// file-backed pages of system libraries (WebKitGTK itself is ~130 MB of
// those, shared with any other WebKitGTK app and reclaimable) is printed too,
// for information.

import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const exe = process.platform === 'win32' ? 'rowboat.exe' : 'rowboat';

/** The release binary, wherever `tauri build [--target …]` put it. */
function findBinary() {
  const target = join(root, 'src-tauri/target');
  const candidates = [join(target, 'release', exe)];
  if (existsSync(target)) {
    // With `--target universal-apple-darwin` there is one directory per
    // architecture. Measure the one that runs natively here (never the other
    // architecture under emulation), in an order that does not depend on the
    // filesystem's directory listing.
    const native = process.arch === 'arm64' ? 'aarch64-' : 'x86_64-';
    const triples = readdirSync(target).sort();
    for (const entry of triples.filter((e) => e.startsWith(native))) {
      candidates.push(join(target, entry, 'release', exe));
    }
    for (const entry of triples.filter((e) => !e.startsWith(native))) {
      candidates.push(join(target, entry, 'release', exe));
    }
  }
  return candidates.find(existsSync) ?? candidates[0];
}
const binary = process.argv[2] ?? findBinary();
const RUNS = Number(process.env.STARTUP_RUNS ?? 5);
const STARTUP_BUDGET_MS = Number(process.env.STARTUP_BUDGET_MS ?? 1000);
const MEMORY_BUDGET_MB = Number(
  process.env.IDLE_MEMORY_BUDGET_MB ?? (process.platform === 'win32' ? 200 : 150),
);
const IDLE_MS = 4000;
const TIMEOUT_MS = 60_000;

if (!existsSync(binary)) {
  console.error(`Binary not found: ${binary}\nRun \`npm run tauri build\` first.`);
  process.exit(2);
}

const work = mkdtempSync(join(tmpdir(), 'rowboat-bench-'));

/** Memory in MB for a process and all its descendants (Linux only). */
function treeMemoryMb(rootPid) {
  const parents = new Map();
  for (const entry of readdirSync('/proc')) {
    if (!/^\d+$/.test(entry)) continue;
    try {
      const stat = readFileSync(`/proc/${entry}/stat`, 'utf8');
      // The command name may contain spaces or parentheses; fields follow the last ')'.
      const ppid = Number(stat.slice(stat.lastIndexOf(')') + 2).split(' ')[1]);
      parents.set(Number(entry), ppid);
    } catch {
      // Process exited while we were looking.
    }
  }
  const inTree = (pid) => {
    for (let p = pid; p && p !== 1; p = parents.get(p)) if (p === rootPid) return true;
    return false;
  };
  let total = 0;
  let priv = 0;
  for (const pid of parents.keys()) {
    if (!inTree(pid)) continue;
    try {
      const rollup = readFileSync(`/proc/${pid}/smaps_rollup`, 'utf8');
      const field = (name) =>
        Number(new RegExp(`^${name}:\\s+(\\d+) kB`, 'm').exec(rollup)?.[1] ?? 0);
      total += field('Pss');
      priv += field('Pss_Anon') + field('Pss_Shmem');
    } catch {
      // Gone, or not ours to read.
    }
  }
  return { totalMb: total / 1024, privateMb: priv / 1024 };
}

function launch(index, { sampleMemory }) {
  return new Promise((resolve, reject) => {
    const out = join(work, `run-${index}.json`);
    const child = spawn(binary, [], {
      stdio: 'ignore',
      env: {
        ...process.env,
        ROWBOAT_BENCH_OUT: out,
        // The clock starts here, before the process exists.
        ROWBOAT_BENCH_T0: String(Date.now()),
        ROWBOAT_BENCH_IDLE_MS: sampleMemory ? String(IDLE_MS) : '0',
        // Fresh settings every run: a true first launch, and nothing of the
        // developer's own config is touched.
        ROWBOAT_CONFIG_DIR: join(work, `config-${index}`),
      },
    });
    let memory = null;
    const timeout = setTimeout(() => {
      child.kill();
      reject(new Error(`run ${index}: the app did not report readiness within ${TIMEOUT_MS} ms`));
    }, TIMEOUT_MS);
    const sampler =
      sampleMemory &&
      setInterval(() => {
        // Sample late in the idle window, once the result file exists.
        if (existsSync(out)) memory = treeMemoryMb(child.pid);
      }, 500);
    child.on('error', reject);
    child.on('exit', () => {
      clearTimeout(timeout);
      if (sampler) clearInterval(sampler);
      if (!existsSync(out)) {
        reject(new Error(`run ${index}: the app exited without reporting readiness`));
        return;
      }
      resolve({ ...JSON.parse(readFileSync(out, 'utf8')), memory });
    });
  });
}

const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];

try {
  // One untimed run first: the very first launch after a build pays one-off
  // costs (filesystem cache, webview data directory) that are not cold start.
  await launch('warmup', { sampleMemory: false });

  const times = [];
  for (let i = 0; i < RUNS; i++) {
    const { startupMs } = await launch(i, { sampleMemory: false });
    times.push(startupMs);
  }
  const startup = median(times);
  console.log(`Startup: ${times.map((t) => t.toFixed(0)).join(', ')} ms`);
  console.log(`Median startup: ${startup.toFixed(0)} ms (budget ${STARTUP_BUDGET_MS} ms)`);

  let failed = startup > STARTUP_BUDGET_MS;
  if (failed) console.error('FAIL: cold start is over budget.');

  if (process.platform === 'linux') {
    const { memory } = await launch('memory', { sampleMemory: true });
    if (!memory || memory.privateMb === 0) {
      console.error('FAIL: could not sample idle memory.');
      failed = true;
    } else {
      console.log(
        `Idle memory: ${memory.privateMb.toFixed(0)} MB private (budget ${MEMORY_BUDGET_MB} MB); ` +
          `${memory.totalMb.toFixed(0)} MB including shared system libraries`,
      );
      if (memory.privateMb > MEMORY_BUDGET_MB) {
        console.error('FAIL: idle memory is over budget.');
        failed = true;
      }
    }
  } else {
    console.log('Idle memory: not measured on this platform yet.');
  }

  process.exit(failed ? 1 : 0);
} catch (error) {
  console.error(`FAIL: ${error.message}`);
  process.exit(1);
} finally {
  rmSync(work, { recursive: true, force: true });
}
