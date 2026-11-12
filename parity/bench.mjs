// Performance baseline: Node vs Rust servers on the same projects dir.
// node parity/bench.mjs [--projects <dir>] [--bin rust/target/release/agentwatch.exe] [--idle 30] [--n 15] [--only rust|node]
// Measures: startup to first /api/state, per-route latency (median / p95), working set + private memory, and CPU
// seconds used while idle (no clients; the poll/tick timers are the only work).
import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const val = (n, d) => { const i = process.argv.indexOf(n); return i >= 0 ? process.argv[i + 1] : d; };
const projects = path.resolve(val('--projects', process.env.CLAUDE_PROJECTS_DIR || path.join(os.homedir(), '.claude', 'projects')));
const exe = process.platform === 'win32' ? 'agentwatch.exe' : 'agentwatch';
const bin = path.resolve(ROOT, val('--bin', path.join('rust', 'target', 'release', exe)));
const IDLE = Number(val('--idle', 30));
const N = Number(val('--n', 15));
const only = val('--only', null);

const freePort = () => new Promise((r) => { const s = net.createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => r(p)); }); });
function get(port, p) {
  return new Promise((resolve, reject) => {
    const t = performance.now();
    const s = net.connect(port, '127.0.0.1', () => s.write(`GET ${p} HTTP/1.1\r\nHost: 127.0.0.1:${port}\r\nConnection: close\r\n\r\n`));
    const chunks = [];
    s.on('data', (d) => chunks.push(d));
    s.on('end', () => {
      const b = Buffer.concat(chunks); const i = b.indexOf('\r\n\r\n');
      let body = b.subarray(i + 4);
      if (/transfer-encoding: chunked/i.test(b.subarray(0, i).toString())) {
        const parts = []; let rest = body;
        for (;;) { const j = rest.indexOf('\r\n'); const n = parseInt(rest.subarray(0, j).toString(), 16); if (!n) break; parts.push(rest.subarray(j + 2, j + 2 + n)); rest = rest.subarray(j + 4 + n); }
        body = Buffer.concat(parts);
      }
      resolve({ ms: performance.now() - t, status: Number(b.subarray(9, 12).toString()), body: body.toString(), bytes: b.length });
    });
    s.on('error', reject);
  });
}
function proc(pid) {
  if (process.platform !== 'win32') return null;
  const out = execFileSync('powershell.exe', ['-NoProfile', '-Command', `$p=Get-Process -Id ${pid}; "$($p.TotalProcessorTime.TotalSeconds) $($p.WorkingSet64) $($p.PrivateMemorySize64)"`], { encoding: 'utf8' }).trim().split(' ').map(Number);
  return { cpu: out[0], ws: out[1] / 2 ** 20, priv: out[2] / 2 ** 20 };
}
const stat = (xs) => { const s = [...xs].sort((a, b) => a - b); return { med: s[Math.floor(s.length / 2)], p95: s[Math.min(s.length - 1, Math.floor(s.length * 0.95))] }; };

async function run(name, cmd, args) {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'aw-bench-'));
  const port = await freePort();
  const t0 = performance.now();
  const child = spawn(cmd, [...args, '--port', String(port), '--projects', projects, '--no-toast'], { env: { ...process.env, AGENTWATCH_HOME: home }, stdio: 'ignore' });
  // ready = /api/state lists every project dir that holds a transcript (Node answers mid-scan with partial data)
  const want = fs.readdirSync(projects, { withFileTypes: true }).filter((d) => d.isDirectory() && fs.readdirSync(path.join(projects, d.name)).some((n) => n.endsWith('.jsonl'))).length;
  let first = null; let ready = null;
  for (let i = 0; i < 2400 && !ready; i++) {
    await new Promise((r) => setTimeout(r, 25));
    const r = await get(port, '/api/state').catch(() => null);
    if (r?.status !== 200) continue;
    first ??= performance.now() - t0;
    if (JSON.parse(r.body).projects.length >= want) ready = performance.now() - t0;
  }
  const startup = ready;
  const all = JSON.parse((await get(port, '/api/state?session=all-recent&main=1')).body);
  const biggest = [...all.agents].sort((a, b) => b.eventCount - a.eventCount)[0];
  const routes = {
    'state latest': '/api/state',
    'state all-recent+main': '/api/state?session=all-recent&main=1',
    'collection latest': '/collection',
    'collection all-recent': '/collection?session=all-recent',
    'snapshot.txt all-recent': '/api/collection/snapshot.txt?session=all-recent',
    'agent full (largest)': `/api/agent/${encodeURIComponent(biggest?.id || 'x')}?full=1`,
  };
  const lat = {};
  for (const [k, p] of Object.entries(routes)) {
    const xs = []; let bytes = 0;
    for (let i = 0; i < 2; i++) await get(port, p); // warm-up (Node answers the first main=1 requests mid-scan)
    for (let i = 0; i < N; i++) { const r = await get(port, p); xs.push(r.ms); bytes = r.bytes; }
    lat[k] = { ...stat(xs), kb: Math.round(bytes / 1024) };
  }
  const a = proc(child.pid);
  await new Promise((r) => setTimeout(r, IDLE * 1000));
  const b = proc(child.pid);
  child.kill();
  fs.rmSync(home, { recursive: true, force: true });
  return { name, startup, first, lat, agents: all.agents.length, sessions: all.sessions.length, mem: b && { ws: b.ws, priv: b.priv }, idleCpu: a && b ? (b.cpu - a.cpu) / IDLE : null };
}

const results = [];
if (only !== 'rust') results.push(await run('node', process.execPath, [path.join(ROOT, 'src', 'server.mjs')]));
if (only !== 'node') results.push(await run('rust', bin, ['serve']));
const f = (x, d = 1) => (x == null ? 'n/a' : x.toFixed(d));
console.log(`projects: ${path.basename(projects)} · ${results[0].sessions} sessions listed · ${results[0].agents} agents in all-recent+main · n=${N} · idle window ${IDLE}s\n`);
console.log(['metric', ...results.map((r) => r.name)].join(' | '));
console.log(['first /api/state response (ms)', ...results.map((r) => f(r.first, 0))].join(' | '));
console.log(['startup until fully scanned (ms)', ...results.map((r) => f(r.startup, 0))].join(' | '));
for (const k of Object.keys(results[0].lat)) console.log([`${k} median/p95 ms`, ...results.map((r) => `${f(r.lat[k].med)} / ${f(r.lat[k].p95)} (${r.lat[k].kb} KB)`)].join(' | '));
console.log(['working set (MB)', ...results.map((r) => f(r.mem?.ws))].join(' | '));
console.log(['private memory (MB)', ...results.map((r) => f(r.mem?.priv))].join(' | '));
console.log(['idle CPU (% of one core)', ...results.map((r) => f(r.idleCpu == null ? null : r.idleCpu * 100, 2))].join(' | '));
