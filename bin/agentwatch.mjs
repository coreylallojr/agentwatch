#!/usr/bin/env node
// agentwatch — launch / inspect the Agent Watch dashboard from any folder.
//
//   agentwatch                open the dashboard (starts the local server if needed), focused on this folder's project
//   agentwatch --all          open it with every project visible
//   agentwatch status         print active sessions and subagents in the terminal (no server, no browser)
//   agentwatch snapshot       print the plain-text collection snapshot (handy for piping to an AI)
//   agentwatch stop           stop the background server
//   agentwatch --no-open      make sure the server is running but don't open a browser
//   agentwatch --port <n>     use another port (default 4778)
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { HOME_DIR, DEFAULT_PORT, Store, loadRules, defaultProjectsRoot, openBrowser } from '../src/server.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const flag = (n) => argv.includes(n);
const val = (n) => { const i = argv.indexOf(n); return i >= 0 ? argv[i + 1] : null; };
const cmd = argv.find((a) => !a.startsWith('-') && argv[argv.indexOf(a) - 1] !== '--port') || 'open';
const PID = path.join(HOME_DIR, 'server.json');

const alive = (pid) => { try { process.kill(pid, 0); return true; } catch (e) { return e.code === 'EPERM'; } };
const getJson = (port, p) => new Promise((resolve) => {
  const req = http.get({ host: '127.0.0.1', port, path: p, timeout: 1500 }, (res) => {
    let b = ''; res.on('data', (d) => { b += d; }); res.on('end', () => resolve(res.statusCode === 200 ? b : null));
  });
  req.on('timeout', () => { req.destroy(); resolve(null); });
  req.on('error', () => resolve(null));
});
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function runningPort() {
  try {
    const info = JSON.parse(fs.readFileSync(PID, 'utf8'));
    if (info?.pid && alive(info.pid)) return info;
  } catch { /* none */ }
  return null;
}

async function ensureServer() {
  const want = Number(val('--port') || DEFAULT_PORT);
  const cur = runningPort();
  if (cur && await getJson(cur.port, '/api/state')) return cur.port;
  spawn(process.execPath, [path.join(HERE, '..', 'src', 'server.mjs'), '--port', String(want)], { detached: true, windowsHide: true, stdio: 'ignore' }).unref();
  for (let i = 0; i < 40; i++) {
    await sleep(150);
    if (await getJson(want, '/api/state')) return want;
  }
  throw new Error(`server did not start on port ${want} (busy? try --port <n>)`);
}

const encode = (p) => String(p).replace(/[^a-zA-Z0-9]/g, '-');
const norm = (p) => String(p || '').replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();

// The project whose folder is this one (or the closest parent of it).
function projectForCwd(projects, cwd) {
  const enc = encode(cwd).toLowerCase(); const c = norm(cwd);
  const exact = projects.find((p) => p.key.toLowerCase() === enc || (p.cwd && norm(p.cwd) === c));
  if (exact) return exact;
  let best = null; // otherwise the closest parent folder that has a project
  for (const p of projects) {
    const pc = norm(p.cwd);
    if (pc && c.startsWith(pc + '/') && (!best || pc.length > norm(best.cwd).length)) best = p;
  }
  return best;
}

const ago = (ms) => { const s = Math.max(0, Math.round((Date.now() - ms) / 1000)); return s < 90 ? `${s}s` : s < 5400 ? `${Math.round(s / 60)}m` : `${Math.round(s / 3600)}h`; };

async function status() {
  const store = new Store({ projectsRoot: defaultProjectsRoot(), rules: loadRules() });
  const all = flag('--all') || flag('-a');
  await store.scan();
  const st = await store.state({ session: 'all-recent' });
  const here = projectForCwd(st.projects, process.cwd());
  const sessions = st.sessions.filter((s) => s.mtime > Date.now() - 24 * 3600 * 1000 && (all || !here || s.projectKey === here.key));
  if (!sessions.length) { console.log('No Claude Code sessions in the last 24h' + (here && !all ? ` for ${here.name} (try --all)` : '') + '.'); return; }
  for (const s of sessions) {
    const ag = st.agents.filter((a) => a.sessionId === s.id && a.kind === 'agent');
    const live = ag.filter((a) => a.status !== 'done').length;
    console.log(`${s.project}  ${s.id.slice(0, 8)}  last activity ${ago(s.mtime)} ago  ${ag.length} subagent${ag.length === 1 ? '' : 's'}${live ? `, ${live} active` : ''}`);
    for (const a of ag.slice(0, 12)) console.log(`   ${a.status.padEnd(8)} ${String(a.type || 'agent').padEnd(18)} ${(a.description || a.id).slice(0, 60)}`);
  }
}

async function main() {
  if (flag('-h') || flag('--help') || cmd === 'help') { console.log(fs.readFileSync(fileURLToPath(import.meta.url), 'utf8').split('\n').filter((l) => l.startsWith('//')).slice(1, 9).map((l) => l.slice(3)).join('\n')); return; }
  if (cmd === 'status') return status();
  if (cmd === 'stop') {
    const cur = runningPort();
    if (!cur) { console.log('Agent Watch is not running.'); return; }
    process.kill(cur.pid); console.log(`Stopped Agent Watch (pid ${cur.pid}).`); return;
  }
  const port = await ensureServer();
  if (cmd === 'snapshot') { process.stdout.write((await getJson(port, '/api/collection/snapshot.txt' + (val('--session') ? `?session=${val('--session')}` : ''))) || 'no data\n'); return; }
  let q = '';
  if (!flag('--all')) {
    const state = JSON.parse((await getJson(port, '/api/state?session=all-recent')) || '{}');
    const p = projectForCwd(state.projects || [], process.cwd());
    if (p) q = `?project=${encodeURIComponent(p.key)}`;
  }
  const url = `http://127.0.0.1:${port}/${q}`;
  if (!flag('--no-open')) openBrowser(url);
  console.log(`Agent Watch: ${url}`);
}

main().then(() => process.exit(0), (e) => { console.error(`agentwatch: ${e.message}`); process.exit(1); });
