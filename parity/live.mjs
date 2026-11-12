// Live-update parity: Node and Rust servers watch the same temp projects dir while it is mutated step by step
// (new sessions/agents, late metadata, appends, partial lines, truncation, new project). After each step both must
// report the same state and agent detail; also reports how fast Rust pushes the SSE update.
// node parity/live.mjs [--bin rust/target/release/agentwatch.exe]
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { startServer } from '../src/server.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const val = (n, d) => { const i = process.argv.indexOf(n); return i >= 0 ? process.argv[i + 1] : d; };
const exe = process.platform === 'win32' ? 'agentwatch.exe' : 'agentwatch';
const bin = path.resolve(ROOT, val('--bin', path.join('rust', 'target', 'release', exe)));
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'aw-live-'));
const home = fs.mkdtempSync(path.join(os.tmpdir(), 'aw-live-home-'));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const iso = (o = 0) => new Date(Date.now() + o).toISOString();
let u = 0;
const line = (o) => JSON.stringify({ uuid: `l${++u}`, timestamp: iso(), ...o }) + '\n';

function get(port, p) {
  return new Promise((resolve, reject) => {
    const s = net.connect(port, '127.0.0.1', () => s.write(`GET ${p} HTTP/1.1\r\nHost: 127.0.0.1:${port}\r\nConnection: close\r\n\r\n`));
    const c = []; s.on('data', (d) => c.push(d)); s.on('error', reject);
    s.on('end', () => {
      const b = Buffer.concat(c); const i = b.indexOf('\r\n\r\n'); let body = b.subarray(i + 4);
      if (/transfer-encoding: chunked/i.test(b.subarray(0, i).toString())) {
        const parts = []; let rest = body;
        for (;;) { const j = rest.indexOf('\r\n'); const n = parseInt(rest.subarray(0, j).toString(), 16); if (!n) break; parts.push(rest.subarray(j + 2, j + 2 + n)); rest = rest.subarray(j + 4 + n); }
        body = Buffer.concat(parts);
      }
      resolve(body.toString());
    });
  });
}
const norm = (s) => s.replace(/"(now|startedAt|pid)":[0-9.]+/g, '"$1":0').replace(/"status":"(running|thinking|stale|stuck)"/g, '"status":"live"');
const freePort = () => new Promise((r) => { const s = net.createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => r(p)); }); });

// SSE listener on the Rust server: records arrival times of agent frames
function sse(port, onAgent) {
  const s = net.connect(port, '127.0.0.1', () => s.write(`GET /events HTTP/1.1\r\nHost: 127.0.0.1:${port}\r\n\r\n`));
  s.on('data', (d) => { for (const m of d.toString().matchAll(/"type":"agent","agent":\{"id":"([^"]+)"/g)) onAgent(m[1]); });
  return s;
}

const A = path.join(dir, 'c--live-a'); const sidA = 'aaaa1111-0000-0000-0000-000000000001';
fs.mkdirSync(A, { recursive: true });
fs.writeFileSync(path.join(A, `${sidA}.jsonl`), line({ type: 'user', cwd: 'C:/live/a', message: { content: 'start' } }));

const node = await startServer({ port: 0, projects: dir, pidfile: false, toast: false, root: home });
const rport = await freePort();
const child = spawn(bin, ['serve', '--port', String(rport), '--projects', dir, '--no-toast'], { env: { ...process.env, AGENTWATCH_HOME: home }, stdio: 'ignore' });
for (let i = 0; i < 100; i++) { await sleep(50); if (await get(rport, '/api/state').then(() => true, () => false)) break; }
await get(node.port, '/api/state?session=all-recent&main=1'); await get(rport, '/api/state?session=all-recent&main=1');
let lastSeen = new Map();
const s = sse(rport, (id) => lastSeen.set(id, performance.now()));

const subA = path.join(A, sidA, 'subagents');
const agentFile = (id) => path.join(subA, `agent-${id}.jsonl`);
const steps = [
  ['new agent transcript', 'x1', () => { fs.mkdirSync(subA, { recursive: true }); fs.writeFileSync(agentFile('x1'), line({ type: 'user', agentId: 'x1', message: { content: 'task one' } })); }],
  ['tool call appended', 'x1', () => fs.appendFileSync(agentFile('x1'), line({ type: 'assistant', message: { content: [{ type: 'tool_use', id: 't1', name: 'Bash', input: { command: 'npm test' } }] } }))],
  ['late metadata', 'x1', () => fs.writeFileSync(path.join(subA, 'agent-x1.meta.json'), JSON.stringify({ agentType: 'Explore', description: 'live one' }))],
  ['partial line', null, () => fs.appendFileSync(agentFile('x1'), '{"type":"user","timestamp":"' + iso() + '","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"o')],
  ['line completed', 'x1', () => fs.appendFileSync(agentFile('x1'), 'k"}]}}\n')],
  ['main session grows', null, () => fs.appendFileSync(path.join(A, `${sidA}.jsonl`), line({ type: 'assistant', message: { content: [{ type: 'text', text: 'main says hi' }] } }))],
  ['second agent', 'x2', () => { fs.writeFileSync(agentFile('x2'), line({ type: 'user', agentId: 'x2', message: { content: 'task two' } }) + line({ type: 'assistant', message: { content: [{ type: 'text', text: 'done' }] } })); fs.writeFileSync(path.join(subA, 'agent-x2.meta.json'), JSON.stringify({ agentType: 'Plan', description: 'two', finished: true })); }],
  ['truncate + rewrite', 'x1', () => fs.writeFileSync(agentFile('x1'), line({ type: 'user', agentId: 'x1', message: { content: 'restarted' } }))],
  ['finish via meta', 'x1', () => fs.writeFileSync(path.join(subA, 'agent-x1.meta.json'), JSON.stringify({ agentType: 'Explore', description: 'live one', finished: true }))],
  ['new project + session', null, () => { const B = path.join(dir, 'c--live-b'); fs.mkdirSync(B, { recursive: true }); fs.writeFileSync(path.join(B, 'bbbb2222-0000-0000-0000-000000000002.jsonl'), line({ type: 'user', cwd: 'C:/live/b', message: { content: 'b' } })); }],
];
let bad = 0;
const lat = [];
for (const [name, agent, act] of steps) {
  const t = performance.now();
  act();
  if (agent) { for (let i = 0; i < 60 && !((lastSeen.get(agent) ?? 0) > t); i++) await sleep(10); if ((lastSeen.get(agent) ?? 0) > t) lat.push([name, lastSeen.get(agent) - t]); else lat.push([name, null]); }
  await sleep(1300); // Node polls every 1 s
  const qs = ['/api/state?session=all-recent&main=1', '/api/state', ...['x1', 'x2', `main-${sidA}`].flatMap((id) => [`/api/agent/${id}`, `/api/agent/${id}?full=1`])];
  for (const q of qs) {
    const [a, b] = [norm(await get(node.port, q)), norm(await get(rport, q))];
    if (a !== b) { bad++; let i = 0; while (a[i] === b[i]) i++; console.log(`X after "${name}" ${q} @${i}\n  node ${JSON.stringify(a.slice(i - 40, i + 60))}\n  rust ${JSON.stringify(b.slice(i - 40, i + 60))}`); }
  }
}
s.destroy(); node.close(); child.kill();
await sleep(200);
fs.rmSync(dir, { recursive: true, force: true }); fs.rmSync(home, { recursive: true, force: true });
console.log(lat.map(([n, ms]) => `  ${n}: ${ms == null ? 'no SSE update within 600 ms' : `${ms.toFixed(0)} ms`}`).join('\n'));
console.log(`${steps.length} live steps · ${bad} state differences`);
process.exit(bad ? 1 : 0);
