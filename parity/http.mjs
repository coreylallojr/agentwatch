// Live HTTP parity: start the Node server in-process and the Rust server as a child, hit the same routes on both and
// compare status, content-type, cache-control and body (volatile fields normalised).
// node parity/http.mjs --projects <dir> [--bin rust/target/release/agentwatch.exe]
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { fileURLToPath } from 'node:url';
import { startServer } from '../src/server.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const val = (n, d) => { const i = process.argv.indexOf(n); return i >= 0 ? process.argv[i + 1] : d; };
const projects = path.resolve(val('--projects'));
const exe = process.platform === 'win32' ? 'agentwatch.exe' : 'agentwatch';
const bin = path.resolve(ROOT, val('--bin', path.join('rust', 'target', 'release', exe)));
const home = fs.mkdtempSync(path.join(os.tmpdir(), 'aw-home-'));

function raw(port, method, p, host = `127.0.0.1:${port}`) {
  return new Promise((resolve, reject) => {
    const s = net.connect(port, '127.0.0.1');
    const chunks = [];
    s.on('connect', () => s.write(`${method} ${p} HTTP/1.1\r\nHost: ${host}\r\nConnection: close\r\n\r\n`));
    s.on('data', (d) => chunks.push(d));
    s.on('end', () => {
      const b = Buffer.concat(chunks); const i = b.indexOf('\r\n\r\n');
      const head = b.subarray(0, i).toString('latin1').split('\r\n');
      const status = Number(head[0].split(' ')[1]);
      const h = Object.fromEntries(head.slice(1).map((l) => [l.slice(0, l.indexOf(':')).toLowerCase(), l.slice(l.indexOf(':') + 1).trim()]));
      let body = b.subarray(i + 4);
      if (/chunked/i.test(h['transfer-encoding'] || '')) {
        const parts = []; let rest = body;
        for (;;) { const j = rest.indexOf('\r\n'); const n = parseInt(rest.subarray(0, j).toString(), 16); if (!n) break; parts.push(rest.subarray(j + 2, j + 2 + n)); rest = rest.subarray(j + 4 + n); }
        body = Buffer.concat(parts);
      }
      resolve({ status, ctype: h['content-type'] || '', cache: h['cache-control'] || '', body });
    });
    s.on('error', reject);
    s.setTimeout(20000, () => { s.destroy(); reject(new Error(`timeout ${p}`)); });
  });
}

const norm = (s) => s
  .replace(/"(now|startedAt|pid|mtime)":[0-9.]+/g, '"$1":0')
  .replace(/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z/g, 'ISO')
  .replace(/snapshot(?: version)?[^0-9a-f]{0,40}[0-9a-f]{10}/g, 'snapshot V');

async function freePort() { return new Promise((r) => { const s = net.createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => r(p)); }); }); }

const node = await startServer({ port: 0, projects, pidfile: false, toast: false, root: home });
const rport = await freePort();
const child = spawn(bin, ['serve', '--port', String(rport), '--projects', projects, '--no-toast'], { env: { ...process.env, AGENTWATCH_HOME: home }, stdio: 'ignore' });
let ok = false;
for (let i = 0; i < 100 && !ok; i++) { await new Promise((r) => setTimeout(r, 100)); ok = await raw(rport, 'GET', '/api/state').then((r) => r.status === 200, () => false); }
if (!ok) { console.error('rust server did not start'); process.exit(2); }

const st = JSON.parse((await raw(node.port, 'GET', '/api/state?session=all-recent&main=1')).body.toString());
const routes = [
  ['GET', '/'], ['HEAD', '/'], ['GET', '/index.html'], ['OPTIONS', '/api/state'], ['GET', '/nope'], ['GET', '/api/state'],
  ['GET', '/api/state?session=all-recent'], ['GET', '/api/state?session=all-recent&main=1'], ['GET', '/api/state?session='],
  ['GET', '/collection'], ['GET', '/collection?fragment=1'], ['GET', '/api/collection/snapshot.txt'], ['GET', '/collection?session=all-recent'],
  ['GET', '/api/agent/nope'], ['GET', '/api/agent/%E0%A4%A'], ['GET', '/api/image/nope/0/0'], ['GET', '/api/../api/state'], ['GET', '/api/state?x=1#frag'],
];
for (const p of st.projects) routes.push(['GET', `/api/state?session=latest&project=${encodeURIComponent(p.key)}`], ['GET', `/collection?project=${encodeURIComponent(p.key)}&session=all-recent`]);
for (const a of st.agents) {
  const id = encodeURIComponent(a.id);
  routes.push(['GET', `/api/agent/${id}`], ['GET', `/api/agent/${id}?full=1`], ['GET', `/api/agent/${id}?event=`], ['GET', `/api/agent/${id}?event=abc`]);
  for (let i = 0; i < a.eventCount; i++) { routes.push(['GET', `/api/agent/${id}?event=${i}`]); for (let k = 0; k < 3; k++) routes.push(['GET', `/api/image/${id}/${i}/${k}`]); }
}
let bad = 0;
const hosts = [null, 'evil.com', 'LOCALHOST:1', 'localhost:'];
for (const h of hosts) {
  const a = await raw(node.port, 'GET', '/api/state', h ?? undefined); const b = await raw(rport, 'GET', '/api/state', h ?? undefined);
  if (a.status !== b.status) { bad++; console.log(`X host ${h}: node ${a.status} rust ${b.status}`); }
}
for (const [m, p] of routes) {
  const [a, b] = [await raw(node.port, m, p), await raw(rport, m, p)];
  const isText = /json|text|html/.test(a.ctype);
  const [ab, bb] = isText ? [norm(a.body.toString()), norm(b.body.toString())] : [a.body.toString('base64'), b.body.toString('base64')];
  const same = a.status === b.status && a.ctype === b.ctype && a.cache === b.cache && ab === bb;
  if (!same) {
    bad++;
    let i = 0; while (i < ab.length && ab[i] === bb[i]) i++;
    console.log(`X ${m} ${p}: status ${a.status}/${b.status} ctype ${a.ctype}/${b.ctype} cache ${a.cache}/${b.cache}${ab !== bb ? ` body@${i}\n  node: ${JSON.stringify(ab.slice(Math.max(0, i - 60), i + 80))}\n  rust: ${JSON.stringify(bb.slice(Math.max(0, i - 60), i + 80))}` : ''}`);
  }
}
// SSE handshake
const sse = (port) => new Promise((resolve) => {
  const s = net.connect(port, '127.0.0.1', () => s.write(`GET /events HTTP/1.1\r\nHost: 127.0.0.1:${port}\r\n\r\n`));
  let b = ''; s.on('data', (d) => { b += d; if (b.includes('"type":"hello"')) { s.destroy(); resolve(b); } });
  setTimeout(() => { s.destroy(); resolve(b); }, 3000);
});
const [sa, sb] = [await sse(node.port), await sse(rport)];
const ev = (s) => /text\/event-stream/i.test(s) && /retry: 2000\ndata: \{"type":"hello","startedAt":\d+\}\n\n/.test(s);
if (!ev(sa) || !ev(sb)) { bad++; console.log(`X SSE hello: node ${ev(sa)} rust ${ev(sb)}`); }
console.log(`\n${routes.length + hosts.length + 1} HTTP checks · ${bad} differ`);
node.close(); child.kill(); fs.rmSync(home, { recursive: true, force: true });
process.exit(bad ? 1 : 0);
