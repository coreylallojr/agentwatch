// Regenerates every image and demo transcript used by the README and the landing page, from synthetic data only.
//   cargo build --release --manifest-path rust/Cargo.toml && node docs/tools/screenshots.mjs
// Needs the `playwright` package with a Chromium build (or Microsoft Edge installed).
import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..', '..');
const IMG = path.join(ROOT, 'docs', 'img');
const exe = path.join(ROOT, 'rust', 'target', 'release', process.platform === 'win32' ? 'agentwatch.exe' : 'agentwatch');
fs.mkdirSync(IMG, { recursive: true });
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'aw-shots-'));
const home = path.join(tmp, 'home');
const env = { ...process.env, AGENTWATCH_HOME: home, CLAUDE_PROJECTS_DIR: path.join(tmp, 'projects') };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const esc = (s) => String(s).replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));

const browser = await chromium.launch().catch(() => chromium.launch({ channel: 'msedge' }));
const shot = async (html, file, { width = 1100, selector = 'body', scale = 2 } = {}) => {
  const p = await browser.newPage({ viewport: { width, height: 1800 }, deviceScaleFactor: scale });
  await p.setContent(html, { waitUntil: 'load' });
  await (await p.$(selector)).screenshot({ path: path.join(IMG, file), omitBackground: true });
  await p.close();
};

// 1. the "order confirmed" image the demo browser agent captures
const confirmPng = path.join(HERE, 'demo-confirm.png');
await shot(`<body style="margin:0"><div id="c" style="width:360px;padding:22px;font:15px system-ui;background:#f4f7fb;border:1px solid #d5dde8">
  <div style="font-weight:700;color:#1b7f3b;font-size:18px">✓ Order confirmed</div><div style="margin-top:6px;color:#334">Order #A-1042 · $84.20 · test card ending 4242</div>
  <div style="margin-top:14px;height:8px;background:#dfe7f1;border-radius:4px"><div style="width:100%;height:8px;background:#2f6fed;border-radius:4px"></div></div></div></body>`, '../tools/demo-confirm.png', { selector: '#c', scale: 1 });
if (!fs.existsSync(confirmPng)) throw new Error('demo-confirm.png not written');

// 2. demo workspace + the real binary
execFileSync(process.execPath, [path.join(HERE, 'demo-projects.mjs'), env.CLAUDE_PROJECTS_DIR]);
const freePort = () => new Promise((r) => { const s = net.createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => r(p)); }); });
const port = await freePort();
const server = spawn(exe, ['serve', '--port', String(port), '--no-toast'], { env, stdio: 'ignore' });
const base = `http://127.0.0.1:${port}`;
for (let i = 0; i < 100; i++) { await sleep(100); if (await fetch(`${base}/api/state`).then((r) => r.ok, () => false)) break; }

// 3. dashboard + collection screenshots
const page = await browser.newPage({ viewport: { width: 1360, height: 820 }, deviceScaleFactor: 1.5, colorScheme: 'light' });
for (const [file, hash, scheme] of [['dashboard.png', 'b7e214', 'light'], ['dashboard-report.png', 'a1f3c9', 'light'], ['dashboard-dark.png', 'd90a17', 'dark']]) {
  await page.emulateMedia({ colorScheme: scheme });
  await page.goto(`${base}/#agent=${hash}`);
  await page.waitForSelector('#lanes .lane'); await sleep(1200);
  if (file === 'dashboard-dark.png') { const b = await page.$('text=show full'); if (b) { await b.click(); await sleep(800); } }
  await page.screenshot({ path: path.join(IMG, file) });
}
await page.emulateMedia({ colorScheme: 'light' });
await page.goto(`${base}/collection`); await sleep(800);
await page.screenshot({ path: path.join(IMG, 'collection.png') });
await page.close();

// 4. real CLI output in a terminal window
const status = execFileSync(exe, ['status', '--all'], { env, encoding: 'utf8' }).replace(/\r/g, '').trimEnd();
const term = (title, body) => `<!doctype html><body style="margin:0;background:transparent"><div id="w" style="width:980px;border-radius:10px;overflow:hidden;box-shadow:0 18px 50px #0006;font:14px/1.55 'Cascadia Mono',Consolas,'SF Mono',monospace;background:#0c1220;color:#d6deeb">
  <div style="background:linear-gradient(#2b3a55,#1d2a40);padding:9px 14px;color:#c6d2e8;font:13px system-ui;display:flex;gap:8px;align-items:center">
  <span style="width:12px;height:12px;border-radius:50%;background:#ff5f57;display:inline-block"></span><span style="width:12px;height:12px;border-radius:50%;background:#febc2e;display:inline-block"></span><span style="width:12px;height:12px;border-radius:50%;background:#28c840;display:inline-block"></span><span style="margin-left:8px">${title}</span></div>
  <pre style="margin:0;padding:18px 20px;white-space:pre-wrap">${body}</pre></div></body>`;
const color = (s) => esc(s).replace(/\b(running)\b/g, '<span style="color:#7aa2ff">$1</span>').replace(/\b(thinking)\b/g, '<span style="color:#c792ea">$1</span>')
  .replace(/\b(stale)\b/g, '<span style="color:#ffcb6b">$1</span>').replace(/\b(stuck)\b/g, '<span style="color:#ff6b6b">$1</span>').replace(/\b(done)\b/g, '<span style="color:#7fdb8a">$1</span>');
await shot(term('PowerShell · C:\\code\\acme-shop', `<span style="color:#7fdb8a">PS C:\\code\\acme-shop&gt;</span> agentwatch status\n${color(status)}\n\n<span style="color:#7fdb8a">PS C:\\code\\acme-shop&gt;</span> agentwatch\nAgent Watch: http://127.0.0.1:4778/?project=C--code-acme-shop`), 'terminal-status.png', { selector: '#w' });

// 5. real MCP tool output in a simulated Claude Code session
const mcp = spawn(exe, ['mcp'], { env: { ...env, CLAUDE_PROJECT_DIR: 'C:\\code\\acme-shop' }, stdio: ['pipe', 'pipe', 'ignore'] });
let buf = ''; const wait = new Map(); let id = 0;
mcp.stdout.on('data', (d) => { buf += d; let i; while ((i = buf.indexOf('\n')) >= 0) { const m = JSON.parse(buf.slice(0, i)); buf = buf.slice(i + 1); wait.get(m.id)?.(m); } });
const rpc = (method, params) => new Promise((r) => { const n = ++id; wait.set(n, r); mcp.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: n, method, params }) + '\n'); });
const tool = async (name, args = {}) => (await rpc('tools/call', { name, arguments: args })).result.content[0].text;
await rpc('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'docs', version: '0' } });
const out = {
  status,
  mcpStatus: await tool('agentwatch_status'),
  mcpErrors: await tool('agentwatch_errors'),
  mcpAgent: await tool('agentwatch_agent', { agent_id: 'b7e214', last: 8 }),
};
mcp.stdin.end(); mcp.kill();
const trimTool = (s, n) => esc(s.split('\n').slice(0, n).join('\n'));
const claude = `<!doctype html><style>#w, #w pre, #w b { font-family: 'Cascadia Mono', Consolas, 'SF Mono', monospace }</style><body style="margin:0;background:transparent"><div id="w" style="width:980px;border-radius:10px;overflow:hidden;box-shadow:0 18px 50px #0006;font:14px/1.55 'Cascadia Mono',Consolas,'SF Mono',monospace;background:#11131a;color:#e3e6ee">
  <div style="background:linear-gradient(#30333d,#24262e);padding:9px 14px;color:#c9ccd6;font:13px system-ui">Claude Code · acme-shop</div>
  <div style="padding:18px 20px">
  <div style="color:#8f96a8">&gt; how are the agents doing? anything broken?</div>
  <div style="margin-top:12px"><span style="color:#d6875f">●</span> <b>agentwatch_status</b> <span style="color:#8f96a8">(MCP)</span></div>
  <pre style="margin:4px 0 0 18px;color:#aeb6c8;white-space:pre-wrap;border-left:2px solid #333846;padding-left:12px">${trimTool(out.mcpStatus, 8)}</pre>
  <div style="margin-top:12px"><span style="color:#d6875f">●</span> <b>agentwatch_errors</b> <span style="color:#8f96a8">(MCP)</span></div>
  <pre style="margin:4px 0 0 18px;color:#aeb6c8;white-space:pre-wrap;border-left:2px solid #333846;padding-left:12px">${trimTool(out.mcpErrors, 9)}</pre>
  <div style="margin-top:14px"><span style="color:#e3e6ee">●</span> Nothing is broken. The explorer and the browser test are done. The test writer had one failing run, which it fixed (recovered), and is now running coverage. The reviewer is mid-review. The one blocked call was the browser agent trying a production deploy, which your deploy guard stopped as intended. I'll wait for the tests with <b>agentwatch_wait</b>.</div>
  </div></div></body>`;
await shot(claude, 'claude-mcp.png', { selector: '#w' });
fs.writeFileSync(path.join(ROOT, 'docs', 'assets', 'demo-output.json'), JSON.stringify(out, null, 2) + '\n');

server.kill();
await browser.close();
fs.rmSync(tmp, { recursive: true, force: true });
console.log('wrote', fs.readdirSync(IMG).join(', '));
