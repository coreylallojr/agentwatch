#!/usr/bin/env node
// Agent Watch server — local, read-only, zero-token live view of Claude Code subagent transcripts.
// agentwatch-server [--port 4778] [--projects <dir>] [--session <id>|latest] [--no-toast] [--open [hash]]
import http from 'node:http';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { AgentBuilder, compileRules, DEFAULTS } from './parse.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const HOME_DIR = process.env.AGENTWATCH_HOME || path.join(os.homedir(), '.agentwatch');
export const DEFAULT_PORT = 4778;
const DAY = 24 * 3600 * 1000;

// Root that holds one directory per Claude Code project (encoded cwd). Agent Watch scans ALL of them.
export function defaultProjectsRoot() {
  return process.env.CLAUDE_PROJECTS_DIR || path.join(os.homedir(), '.claude', 'projects');
}

// Best-effort real folder for a session: the "cwd" recorded in the first lines of its transcript.
function readCwd(file) {
  try {
    const fd = fs.openSync(file, 'r');
    try {
      const buf = Buffer.alloc(16384);
      const n = fs.readSync(fd, buf, 0, buf.length, 0);
      const m = /"cwd":"((?:[^"\\]|\\.)*)"/.exec(buf.subarray(0, n).toString('utf8'));
      return m ? JSON.parse('"' + m[1] + '"') : null;
    } finally { fs.closeSync(fd); }
  } catch { return null; }
}

export function parseArgs(argv) {
  const o = { port: DEFAULT_PORT, projects: null, session: 'latest', toast: true, open: null, idleMin: 30, maxIdleH: 8 };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]; const next = () => argv[++i];
    if (a === '--port') o.port = Number(next());
    else if (a === '--projects' || a === '--project') o.projects = next();
    else if (a === '--session') o.session = next();
    else if (a === '--no-toast') o.toast = false;
    else if (a === '--open') { o.open = argv[i + 1] && !argv[i + 1].startsWith('--') ? next() : ''; }
    else if (a === '--idle-min') o.idleMin = Number(next());
    else if (a === '--max-idle-h') o.maxIdleH = Number(next());
  }
  return o;
}

// User rules (~/.agentwatch/rules.json) override the bundled defaults.
export function loadRules(file) {
  const files = file ? [file] : [path.join(HOME_DIR, 'rules.json'), path.join(HERE, '..', 'rules.json')];
  for (const f of files) {
    try { return compileRules(JSON.parse(fs.readFileSync(f, 'utf8'))); } catch { /* try next */ }
  }
  return [];
}

// ---------------- watcher / store ----------------
export class Store {
  constructor({ projectsRoot, rules = [], staleSec = DEFAULTS.staleSec, stuckSec = DEFAULTS.stuckSec, onAgent = () => {} }) {
    this.projectsRoot = projectsRoot; this.rules = rules; this.staleSec = staleSec; this.stuckSec = stuckSec;
    this.onAgent = onAgent;
    this.sessions = new Map();   // id -> {id, mainPath, mtime, tracked, wantMain}
    this.files = new Map();      // filePath -> rec
    this.byId = new Map();       // agent id -> rec
    this.lastFileChange = Date.now();
    this.scanning = false; this.rescan = false;
  }

  opts(now = Date.now()) { return { now, staleSec: this.staleSec, stuckSec: this.stuckSec, rules: this.rules }; }

  async scan() {
    if (this.scanning) { this.rescan = true; return; }
    this.scanning = true;
    try {
      do { this.rescan = false; await this.scanOnce(); } while (this.rescan);
    } finally { this.scanning = false; }
  }

  async scanOnce() {
    let dirs = [];
    try { dirs = (await fsp.readdir(this.projectsRoot, { withFileTypes: true })).filter((d) => d.isDirectory()).map((d) => d.name); } catch { return; }
    const now = Date.now();
    for (const dirName of dirs) {
      const projectDir = path.join(this.projectsRoot, dirName);
      let names = [];
      try { names = await fsp.readdir(projectDir); } catch { continue; }
      for (const n of names) {
        if (!n.endsWith('.jsonl')) continue;
        const id = n.slice(0, -6);
        const mainPath = path.join(projectDir, n);
        let st; try { st = await fsp.stat(mainPath); } catch { continue; }
        let s = this.sessions.get(id);
        if (!s) { s = { id, mainPath, projectDir, projectKey: dirName, cwd: null, mtime: 0, tracked: false, wantMain: false }; this.sessions.set(id, s); }
        s.mainMtime = st.mtimeMs; s.mtime = Math.max(s.mtime, st.mtimeMs);
        if (!s.cwd) { s.cwd = readCwd(mainPath); if (s.cwd) s.project = path.basename(s.cwd.replace(/[\\/]+$/, '')) || s.cwd; }
        if (!s.tracked && now - st.mtimeMs < DAY) s.tracked = true;
      }
    }
    for (const s of this.sessions.values()) {
      if (!s.tracked) continue;
      await this.scanSubagents(s);
      if (s.wantMain) await this.readFile(s.mainPath, { kind: 'main', sessionId: s.id, id: `main-${s.id}` });
    }
  }

  async scanSubagents(s) {
    const dir = path.join(s.projectDir, s.id, 'subagents');
    let names; try { names = await fsp.readdir(dir); } catch { return; }
    for (const n of names) {
      const m = /^agent-(.+)\.jsonl$/.exec(n);
      if (!m) continue;
      await this.readFile(path.join(dir, n), { kind: 'agent', sessionId: s.id, id: m[1], metaPath: path.join(dir, `agent-${m[1]}.meta.json`) });
    }
  }

  async readFile(file, info) {
    let rec = this.files.get(file);
    if (!rec) {
      rec = { ...info, file, offset: 0, buf: Buffer.alloc(0), metaMtime: 0, builder: null, lastStatus: null };
      rec.builder = new AgentBuilder({ id: info.id, kind: info.kind, sessionId: info.sessionId });
      this.files.set(file, rec); this.byId.set(info.id, rec);
    }
    let changed = false;
    if (rec.metaPath) {
      try {
        const st = await fsp.stat(rec.metaPath);
        if (st.mtimeMs !== rec.metaMtime) {
          rec.metaMtime = st.mtimeMs;
          rec.builder.meta = JSON.parse(await fsp.readFile(rec.metaPath, 'utf8'));
          changed = true;
        }
      } catch { /* meta not yet written or mid-write */ }
    }
    let st; try { st = await fsp.stat(file); } catch { return; }
    if (st.size < rec.offset) { // truncated/rewritten: start over
      const meta = rec.builder.meta;
      rec.builder = new AgentBuilder({ id: rec.id, kind: rec.kind, sessionId: rec.sessionId }); rec.builder.meta = meta;
      rec.offset = 0; rec.buf = Buffer.alloc(0);
    }
    if (st.size > rec.offset) {
      const fh = await fsp.open(file, 'r');
      try {
        const len = st.size - rec.offset;
        const chunk = Buffer.alloc(len);
        const { bytesRead } = await fh.read(chunk, 0, len, rec.offset);
        rec.offset += bytesRead;
        let buf = Buffer.concat([rec.buf, chunk.subarray(0, bytesRead)]);
        const nl = buf.lastIndexOf(0x0a);
        if (nl >= 0) {
          const text = buf.subarray(0, nl).toString('utf8');
          rec.buf = Buffer.from(buf.subarray(nl + 1));
          for (const line of text.split('\n')) {
            if (!line.trim()) continue;
            let r; try { r = JSON.parse(line); } catch { continue; }
            if (rec.builder.push(r)) changed = true;
          }
        } else rec.buf = buf; // partial line still being written
      } finally { await fh.close(); }
      this.lastFileChange = Date.now();
      const s = this.sessions.get(rec.sessionId);
      if (s) s.mtime = Math.max(s.mtime, st.mtimeMs);
    }
    if (changed) this.emit(rec);
  }

  emit(rec) {
    const snap = rec.builder.snapshot(this.opts());
    rec.lastStatus = snap.status;
    this.onAgent(snap, rec);
  }

  // periodic re-evaluation for time-based status (stale/stuck)
  tick() {
    const o = this.opts();
    for (const rec of this.files.values()) {
      if (rec.builder.meta?.finished && rec.lastStatus === 'done') continue;
      const snap = rec.builder.snapshot(o);
      if (snap.status !== rec.lastStatus) { rec.lastStatus = snap.status; this.onAgent(snap, rec); }
    }
  }

  latestSessionId(project = null) {
    let best = null;
    for (const s of this.sessions.values()) {
      if (project && s.projectKey !== project) continue;
      if (!best || s.mtime > best.mtime) best = s;
    }
    return best?.id ?? null;
  }

  async ensureTracked(id, wantMain) {
    const s = this.sessions.get(id);
    if (!s) return;
    let need = false;
    if (!s.tracked) { s.tracked = true; need = true; }
    if (wantMain && !s.wantMain) { s.wantMain = true; need = true; }
    if (need) await this.scan();
  }

  async state({ session = 'latest', main = false, project = null } = {}) {
    if (!this.sessions.size) await this.scan();
    const now = Date.now();
    let ids;
    if (session === 'all-recent') ids = [...this.sessions.values()].filter((s) => now - s.mtime < DAY && (!project || s.projectKey === project)).map((s) => s.id);
    else ids = [session === 'latest' || !session ? this.latestSessionId(project) : session].filter(Boolean);
    for (const id of ids) await this.ensureTracked(id, main);
    const o = this.opts(now);
    const agents = [];
    for (const rec of this.files.values()) {
      if (!ids.includes(rec.sessionId)) continue;
      if (rec.kind === 'main' && !main) continue;
      agents.push(rec.builder.snapshot(o));
    }
    agents.sort((a, b) => (b.lastEventAt ?? 0) - (a.lastEventAt ?? 0));
    const sessions = [...this.sessions.values()].filter((s) => !project || s.projectKey === project).sort((a, b) => b.mtime - a.mtime).slice(0, 40).map((s) => ({
      id: s.id, mtime: s.mtime, project: s.project || s.projectKey, projectKey: s.projectKey, cwd: s.cwd,
      agents: [...this.files.values()].filter((r) => r.sessionId === s.id && r.kind === 'agent').length,
      tracked: s.tracked,
    }));
    const projects = [...new Map([...this.sessions.values()].map((s) => [s.projectKey, { key: s.projectKey, name: s.project || s.projectKey, cwd: s.cwd, mtime: 0 }])).values()];
    for (const p of projects) p.mtime = Math.max(0, ...[...this.sessions.values()].filter((s) => s.projectKey === p.key).map((s) => s.mtime));
    projects.sort((a, b) => b.mtime - a.mtime);
    return { now, projects, selected: session === 'all-recent' ? 'all-recent' : ids[0] ?? null, sessionIds: ids, sessions, agents, staleSec: this.staleSec, stuckSec: this.stuckSec };
  }

  agent(id, eventIdx, { full = false } = {}) {
    const rec = this.byId.get(id);
    if (!rec) return null;
    if (full) {
      // collection page: every event with its full (redacted, capped) output and images merged in
      const d = rec.builder.detail(this.opts());
      d.events = d.events.map((e, i) => { const f = rec.builder.full.get(i); return f ? { ...e, output: f.output ?? e.output, images: f.images || [] } : e; });
      return d;
    }
    if (eventIdx != null) {
      const ev = rec.builder.events[eventIdx];
      if (!ev) return null;
      const full = rec.builder.full.get(eventIdx) || {};
      const { rawPathHints, ...e } = ev;
      return { event: { ...e, output: full.output ?? e.output, images: full.images || [] } };
    }
    return rec.builder.detail(this.opts());
  }
}

// ---------------- notifications ----------------
function showToast(title, text) {
  if (process.platform !== 'win32') return;
  const script = [
    'Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing;',
    '$n = New-Object System.Windows.Forms.NotifyIcon; $n.Icon = [System.Drawing.SystemIcons]::Information; $n.Visible = $true;',
    "$n.ShowBalloonTip(8000, $env:AW_TITLE, $env:AW_TEXT, 'Info'); Start-Sleep -Seconds 9; $n.Dispose()",
  ].join(' ');
  try {
    spawn('powershell.exe', ['-NoProfile', '-NonInteractive', '-WindowStyle', 'Hidden', '-Command', script], {
      env: { ...process.env, AW_TITLE: title.slice(0, 60), AW_TEXT: text.slice(0, 200) },
      windowsHide: true, detached: true, stdio: 'ignore',
    }).unref();
  } catch { /* ignore */ }
}

export function openBrowser(url) {
  try {
    if (process.platform === 'win32') {
      spawn('cmd.exe', ['/d', '/s', '/c', `start "" "${url}"`], { windowsVerbatimArguments: true, windowsHide: true, detached: true, stdio: 'ignore' }).unref();
    } else {
      spawn(process.platform === 'darwin' ? 'open' : 'xdg-open', [url], { detached: true, stdio: 'ignore' }).unref();
    }
  } catch { /* ignore */ }
}

// ---------------- HTTP ----------------
export async function startServer(options = {}) {
  const o = { port: DEFAULT_PORT, session: 'latest', toast: true, open: null, idleMin: 30, maxIdleH: 8, pidfile: true, root: HOME_DIR, ...options };
  const projectsRoot = o.projects ? path.resolve(o.projects) : defaultProjectsRoot();
  const startedAt = Date.now();
  const clients = new Set();
  let lastClientAt = Date.now();
  const toastLog = new Map(); // agent|rule -> ms
  const seenFocus = new Set();

  const broadcast = (obj) => {
    const data = `data: ${JSON.stringify(obj)}\n\n`;
    for (const res of clients) { try { res.write(data); } catch { /* dropped */ } }
  };

  const store = new Store({
    projectsRoot, rules: o.rules ?? loadRules(),
    onAgent: (agent) => {
      broadcast({ type: 'agent', agent });
      for (const h of agent.focus || []) {
        const key = `${agent.id}|${h.rule}|${h.idx}`;
        if (seenFocus.has(key)) continue;
        seenFocus.add(key);
        const fresh = h.rule === 'stuck' || h.idx === -1
          ? (agent.lastEventAt ?? 0) > startedAt - 60 * 60 * 1000
          : (h.t ?? 0) >= startedAt - 10_000;
        if (!fresh) continue;
        broadcast({ type: 'focus', agentId: agent.id, sessionId: agent.sessionId, rule: h.rule, action: h.action, summary: h.summary, t: h.t });
        if (h.action === 'toast' && o.toast) {
          const tk = `${agent.id}|${h.rule}`;
          if (Date.now() - (toastLog.get(tk) ?? 0) < 5 * 60 * 1000) continue;
          toastLog.set(tk, Date.now());
          showToast(`Agent Watch: ${h.rule}`, `${agent.type} - ${agent.description || agent.id}`);
        }
      }
    },
  });

  const indexPath = path.join(HERE, 'index.html');
  const server = http.createServer(async (req, res) => {
    try {
      const host = String(req.headers.host || '');
      if (!/^(127\.0\.0\.1|localhost)(:\d+)?$/i.test(host)) { res.writeHead(403).end('forbidden'); return; }
      if (req.method !== 'GET' && req.method !== 'HEAD') { res.writeHead(405).end(); return; }
      const url = new URL(req.url, 'http://127.0.0.1');
      const json = (code, body) => { res.writeHead(code, { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store' }); res.end(JSON.stringify(body)); };
      if (url.pathname === '/' || url.pathname === '/index.html') {
        res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' });
        res.end(await fsp.readFile(indexPath));
      } else if (url.pathname === '/collection' || url.pathname === '/api/collection/snapshot.txt') {
        // Collection page: every agent (main session included), every event, fully rendered server-side.
        const { buildModel, renderPage, renderBody, snapshotText } = await import('./collection.mjs');
        const state = await store.state({ session: url.searchParams.get('session') || o.session, main: true, project: url.searchParams.get('project') || null });
        const details = Object.fromEntries(state.agents.map((a) => [a.id, store.agent(a.id, null, { full: true })]));
        const model = buildModel(state, details);
        if (url.pathname.endsWith('.txt')) { res.writeHead(200, { 'content-type': 'text/plain; charset=utf-8', 'cache-control': 'no-store' }); res.end(snapshotText(model)); return; }
        res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' });
        res.end(url.searchParams.get('fragment') === '1' ? renderBody(model) : renderPage(model));
      } else if (url.pathname.startsWith('/api/image/')) {
        const [id, ev, k] = url.pathname.slice('/api/image/'.length).split('/').map(decodeURIComponent);
        const im = store.agent(id, Number(ev))?.event?.images?.[Number(k)];
        if (!im?.data) { json(404, { error: 'not found' }); return; }
        res.writeHead(200, { 'content-type': /^image\/(png|jpeg|gif|webp)$/.test(im.mediaType) ? im.mediaType : 'application/octet-stream', 'cache-control': 'max-age=3600' });
        res.end(Buffer.from(im.data, 'base64'));
      } else if (url.pathname === '/api/state') {
        json(200, { ...(await store.state({ session: url.searchParams.get('session') || o.session, main: url.searchParams.get('main') === '1', project: url.searchParams.get('project') || null })), server: { startedAt, pid: process.pid } });
      } else if (url.pathname.startsWith('/api/agent/')) {
        const id = decodeURIComponent(url.pathname.slice('/api/agent/'.length));
        const ev = url.searchParams.get('event');
        const body = store.agent(id, ev != null ? Number(ev) : null, { full: url.searchParams.get('full') === '1' });
        body ? json(200, body) : json(404, { error: 'not found' });
      } else if (url.pathname === '/events') {
        res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-store', connection: 'keep-alive' });
        res.write(`retry: 2000\ndata: ${JSON.stringify({ type: 'hello', startedAt })}\n\n`);
        clients.add(res); lastClientAt = Date.now();
        req.on('close', () => { clients.delete(res); lastClientAt = Date.now(); });
      } else json(404, { error: 'not found' });
    } catch (e) {
      try { res.writeHead(500).end(String(e?.message || e)); } catch { /* ignore */ }
    }
  });

  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(o.port, '127.0.0.1', () => { server.off('error', reject); resolve(); });
  });
  const port = server.address().port;

  await store.scan();
  let watcher = null;
  let debounce = null;
  try {
    watcher = fs.watch(projectsRoot, { recursive: true }, () => {
      clearTimeout(debounce); debounce = setTimeout(() => store.scan().catch(() => {}), 60);
    });
    watcher.on('error', () => {});
  } catch { /* polling only */ }
  const poll = setInterval(() => store.scan().catch(() => {}), 1000);
  const tick = setInterval(() => store.tick(), 5000);
  const ping = setInterval(() => { for (const r of clients) { try { r.write(': ping\n\n'); } catch { /* */ } } if (clients.size) lastClientAt = Date.now(); }, 15000);
  const idle = setInterval(() => {
    const now = Date.now();
    const quietFiles = now - store.lastFileChange;
    if (clients.size === 0 && now - lastClientAt >= o.idleMin * 60_000 && quietFiles >= o.idleMin * 60_000) close('idle');
    else if (quietFiles >= o.maxIdleH * 3600_000) close('idle-max');
  }, 60_000);
  for (const t of [poll, tick, ping, idle]) t.unref?.();

  const pidPath = path.join(o.root, 'server.json');
  if (o.pidfile) {
    try {
      fs.mkdirSync(path.dirname(pidPath), { recursive: true });
      fs.writeFileSync(pidPath, JSON.stringify({ pid: process.pid, port, startedAt: new Date(startedAt).toISOString() }, null, 2));
    } catch { /* ignore */ }
  }

  let closed = false;
  function close(reason) {
    if (closed) return; closed = true;
    for (const t of [poll, tick, ping, idle]) clearInterval(t);
    try { watcher?.close(); } catch { /* */ }
    for (const r of clients) { try { r.end(); } catch { /* */ } }
    server.close();
    if (o.pidfile) {
      try { const cur = JSON.parse(fs.readFileSync(pidPath, 'utf8')); if (cur.pid === process.pid) fs.unlinkSync(pidPath); } catch { /* */ }
    }
    if (o.exitOnClose) process.exit(0);
    return reason;
  }

  if (o.open != null) openBrowser(`http://127.0.0.1:${port}/${o.open ? (o.open.startsWith('#') ? o.open : `#${o.open}`) : ''}`);
  return { server, port, address: server.address(), store, close, projectsRoot };
}

function probe(port) {
  return new Promise((resolve) => {
    const req = http.get({ host: '127.0.0.1', port, path: '/api/state', timeout: 800 }, (res) => { res.resume(); resolve(res.statusCode === 200); });
    req.on('timeout', () => { req.destroy(); resolve(false); });
    req.on('error', () => resolve(false));
  });
}

const isMain = process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href;
if (isMain) {
  const args = parseArgs(process.argv.slice(2));
  try {
    const s = await startServer({ ...args, exitOnClose: true });
    process.stderr.write(`Agent Watch on http://127.0.0.1:${s.port}/  (watching all projects)\n`);
    for (const sig of ['SIGINT', 'SIGTERM', 'SIGBREAK']) process.on(sig, () => s.close(sig));
  } catch (e) {
    if (e?.code === 'EADDRINUSE') {
      const ours = await probe(args.port);
      process.stderr.write(ours
        ? `Agent Watch already running on http://127.0.0.1:${args.port}/\n`
        : `Port ${args.port} is busy (another program). Use --port <n>.\n`);
      process.exit(ours ? 0 : 3);
    }
    process.stderr.write(`Agent Watch failed: ${e?.message || e}\n`);
    process.exit(1);
  }
}
