// Differential parity check: Node (oracle) vs Rust (`agentwatch __dump`) over the same projects dir and clock.
// node parity/run.mjs --projects <dir> [--now <ms>] [--bin rust/target/debug/agentwatch.exe] [--show 5]
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const val = (n, d) => { const i = process.argv.indexOf(n); return i >= 0 ? process.argv[i + 1] : d; };
const projects = path.resolve(val('--projects'));
const now = String(val('--now', Date.now()));
const exe = process.platform === 'win32' ? 'agentwatch.exe' : 'agentwatch';
const bin = path.resolve(ROOT, val('--bin', path.join('rust', 'target', 'debug', exe)));
const big = { maxBuffer: 2 ** 31 - 1, encoding: 'utf8' };

const t0 = performance.now();
const node = JSON.parse(execFileSync(process.execPath, [path.join(ROOT, 'parity', 'oracle.mjs'), '--projects', projects, '--now', now], big));
const t1 = performance.now();
const rust = JSON.parse(execFileSync(bin, ['__dump', '--projects', projects, '--now', now], big));
const t2 = performance.now();

const keys = new Set([...Object.keys(node), ...Object.keys(rust)]);
const diffs = [];
for (const k of keys) {
  const a = node[k]; const b = rust[k];
  if (a === b) continue;
  if (a == null || b == null) { diffs.push({ k, why: a == null ? 'only in rust' : 'only in node' }); continue; }
  let i = 0; while (i < a.length && a[i] === b[i]) i++;
  diffs.push({ k, why: `first difference at char ${i}`, node: JSON.stringify(a.slice(Math.max(0, i - 80), i + 120)), rust: JSON.stringify(b.slice(Math.max(0, i - 80), i + 120)) });
}
const show = Number(val('--show', 5));
for (const d of diffs.slice(0, show)) console.log(`\nX ${d.k}: ${d.why}${d.node ? `\n  node: ${d.node}\n  rust: ${d.rust}` : ''}`);
const kinds = {};
for (const d of diffs) { const k = d.k.split('|')[0]; kinds[k] = (kinds[k] || 0) + 1; }
console.log(`\n${keys.size} surfaces compared · ${diffs.length} differ ${JSON.stringify(kinds)} · node ${(t1 - t0).toFixed(0)} ms · rust ${(t2 - t1).toFixed(0)} ms`);
process.exit(diffs.length ? 1 : 0);
