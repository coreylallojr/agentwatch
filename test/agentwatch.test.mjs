// node --test tools/agentwatch/
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { parseAgentFile, summarize, redact, splitLines, compileRules, computeStatus, AgentBuilder, projectDirName } from '../src/parse.mjs';
import { startServer, loadRules } from '../src/server.mjs';

const T0 = Date.parse('2026-01-01T10:00:00.000Z');
const ts = (s) => new Date(T0 + s * 1000).toISOString();
let n = 0;
const line = (o) => JSON.stringify({ uuid: `u${++n}`, agentId: 'abc123', ...o });
const FAKE_JWT = ['eyJhbGciOiJIUzI1NiJ9', 'eyJzdWIiOiJ4In0', 'c2lnbmF0dXJlLXZhbHVl'].join('.');

function fixture({ finishedTool = true } = {}) {
  const L = [
    line({ type: 'user', timestamp: ts(0), message: { role: 'user', content: 'Synthetic task. Contact test.person@example.com about card 4111 1111 1111 1111.' } }),
    line({ type: 'attachment', timestamp: ts(0.5), attachment: {} }),
    line({ type: 'assistant', timestamp: ts(1), message: { content: [{ type: 'thinking', thinking: '' }] } }),
    line({ type: 'assistant', timestamp: ts(2), message: { content: [{ type: 'tool_use', id: 't1', name: 'Bash', input: { command: 'npm test\necho done' } }] } }),
    line({ type: 'user', timestamp: ts(5), message: { content: [{ type: 'tool_result', tool_use_id: 't1', content: `ok token ${FAKE_JWT}` }] } }),
    line({ type: 'assistant', timestamp: ts(6), message: { content: [{ type: 'tool_use', id: 't2', name: 'Read', input: { file_path: 'C:\\x\\apps\\web-app\\src\\engine\\classify.js', offset: 120, limit: 60 } }] } }),
    line({ type: 'user', timestamp: ts(7), message: { content: [{ type: 'tool_result', tool_use_id: 't2', content: [{ type: 'text', text: 'x'.repeat(5000) }] }] } }),
    line({ type: 'assistant', timestamp: ts(8), message: { content: [{ type: 'tool_use', id: 't3', name: 'Bash', input: { command: 'node tools/api.mjs GET /x' } }] } }),
    line({ type: 'user', timestamp: ts(9), message: { content: [{ type: 'tool_result', tool_use_id: 't3', is_error: true, content: '[policy guard] blocked: synthetic' }] } }),
    line({ type: 'assistant', timestamp: ts(10), message: { content: [{ type: 'tool_use', id: 't4', name: 'mcp__playwright__browser_click', input: { element: 'Submit button', ref: 'e1' } }] } }),
  ];
  if (finishedTool) {
    L.push(line({ type: 'user', timestamp: ts(11), message: { content: [{ type: 'tool_result', tool_use_id: 't4', content: [{ type: 'text', text: 'clicked' }, { type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'AAAA' } }] }] } }));
    L.push(line({ type: 'assistant', timestamp: ts(12), message: { content: [{ type: 'text', text: '## Report\nAll **good**. Mail admin@example.org' }] } }));
  }
  return L.join('\n') + '\n{"type":"assistant","timest'; // partially-written last line
}

const META = { agentType: 'Explore', description: 'Synthetic run', toolUseId: 'toolu_x', spawnDepth: 1 };

test('splitLines keeps a partial trailing line', () => {
  const { records, partial } = splitLines('{"a":1}\n{"b":2}\n{"c":');
  assert.equal(records.length, 2); assert.equal(partial, '{"c":');
});

test('parse: lane fields, pairing, durations, images', () => {
  const a = parseAgentFile(fixture(), { ...META, finished: true }, { id: 'abc123', now: T0 + 13000 });
  assert.equal(a.id, 'abc123'); assert.equal(a.type, 'Explore'); assert.equal(a.description, 'Synthetic run');
  assert.equal(a.status, 'done'); assert.equal(a.toolCount, 4);
  assert.equal(a.startedAt, T0); assert.equal(a.lastEventAt, T0 + 12000);
  const tools = a.events.filter((e) => e.kind === 'tool');
  assert.equal(tools.length, 4);
  assert.ok(tools.every((t) => t.done));
  assert.equal(tools[0].durationMs, 3000);
  assert.equal(tools[2].isError, true); assert.equal(tools[2].deny, true);
  assert.equal(tools[3].images.length, 1);
  assert.ok(a.events.some((e) => e.kind === 'thinking'));
  assert.ok(a.events.some((e) => e.kind === 'text'));
  // truncation: 5000-char output trimmed to head+tail with marker
  assert.match(tools[1].output, /chars trimmed/); assert.ok(tools[1].output.length < 2200);
});

test('summaries', () => {
  assert.equal(summarize('Bash', { command: 'npm test' }), 'Bash: npm test');
  assert.equal(summarize('Read', { file_path: 'C:\\a\\b\\classify.js', offset: 120, limit: 60 }), 'Read: classify.js:120-180');
  assert.equal(summarize('Edit', { file_path: '/a/b/file.mjs' }), 'Edit: file.mjs');
  assert.equal(summarize('Write', { file_path: '/a/b/new.md' }), 'Write: new.md');
  assert.equal(summarize('Grep', { pattern: 'foo', path: '/a/src/engine' }), 'Grep: "foo" in src/engine');
  assert.equal(summarize('Glob', { pattern: '**/*.ts' }), 'Glob: **/*.ts');
  assert.equal(summarize('mcp__playwright__browser_click', { element: 'Send' }), 'Playwright click "Send"');
  assert.equal(summarize('Agent', { description: 'Explore repo' }), 'Agent: Explore repo');
  assert.equal(summarize('WebSearch', { query: 'node fs.watch' }), 'WebSearch: node fs.watch');
});

test('status: running / thinking / stale / stuck with injected now', () => {
  const txt = fixture({ finishedTool: false });
  assert.equal(parseAgentFile(txt, META, { id: 'a', now: T0 + 11000 }).status, 'running');
  const b = new AgentBuilder({ id: 'b' });
  for (const r of splitLines(fixture()).records) b.push(r);
  assert.equal(computeStatus(b, {}, T0 + 13000), 'thinking');
  assert.equal(computeStatus(b, {}, T0 + 12000 + 100_000), 'stale');
  assert.equal(computeStatus(b, {}, T0 + 12000 + 300_000), 'stuck');
  assert.equal(computeStatus(b, { finished: true }, T0 + 12000 + 300_000), 'done');
});

test('focus rules: browser automation, hook block, stuck', () => {
  const rules = loadRules();
  const a = parseAgentFile(fixture(), META, { id: 'a', now: T0 + 12000 + 300_000, rules });
  const names = new Set(a.focus.map((f) => f.rule));
  for (const r of ['browser-automation', 'blocked-by-hook', 'stuck']) assert.ok(names.has(r), r);
  assert.equal(a.focus.find((f) => f.rule === 'blocked-by-hook').action, 'toast');
  assert.equal(compileRules([{ name: 'x', match: { tool: '^Bash$' } }])[0].tool.test('Bash'), true);
});

test('redaction: email, JWT, card, tokens absent from parsed output', () => {
  const a = parseAgentFile(fixture(), { ...META, finished: true }, { id: 'a' });
  const all = JSON.stringify(a);
  assert.ok(!all.includes('test.person@example.com'));
  assert.ok(!all.includes('admin@example.org'));
  assert.ok(!all.includes(FAKE_JWT.slice(0, 30)));
  assert.ok(!all.includes('4111 1111 1111 1111'));
  const r = redact('SERVICE_API_TOKEN=abcdef123456 and X-Api-Token: zzzz9999yyyy Authorization: Bearer abcdefghijklmnop1234 '
    + 'k=0123456789abcdef0123456789abcdef01 s=Ab3dEf7hIj1lMn0pQr8tUv6xYz2aBc4dEf');
  assert.ok(!/abcdef123456|zzzz9999yyyy|abcdefghijklmnop1234|0123456789abcdef0123456789abcdef01|Ab3dEf7hIj1lMn0pQr8tUv6xYz2aBc4dEf/.test(r), r);
  // ordinary ids/paths survive
  assert.equal(redact('87764ec4-8412-412e-b9f2-25aafa97e040'), '87764ec4-8412-412e-b9f2-25aafa97e040');
  assert.equal(redact('c--Users-Some-Person-Documents-My-Workspace'), 'c--Users-Some-Person-Documents-My-Workspace');
});

test('projectDirName mirrors Claude path encoding', () => {
  assert.equal(projectDirName('c:\\Users\\A.B\\Documents\\My-Workspace'), 'c--Users-A-B-Documents-My-Workspace');
});

function tmpProject() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'aw-'));
  const proj = path.join(dir, 'proj'); const sid = '11111111-2222-3333-4444-555555555555';
  const sub = path.join(proj, sid, 'subagents');
  fs.mkdirSync(sub, { recursive: true });
  fs.writeFileSync(path.join(proj, `${sid}.jsonl`), `${JSON.stringify({ type: 'user', timestamp: ts(0), message: { content: 'hi' } })}\n`);
  fs.writeFileSync(path.join(sub, 'agent-abc123.jsonl'), fixture());
  fs.writeFileSync(path.join(sub, 'agent-abc123.meta.json'), JSON.stringify({ ...META, finished: true }));
  fs.writeFileSync(path.join(sub, 'agent-def456.jsonl'), fixture({ finishedTool: false }));
  fs.writeFileSync(path.join(sub, 'agent-def456.meta.json'), JSON.stringify(META));
  return { dir, proj, sid, sub };
}

const get = (port, p) => new Promise((resolve, reject) => {
  http.get({ host: '127.0.0.1', port, path: p }, (res) => { let b = ''; res.on('data', (d) => { b += d; }); res.on('end', () => resolve({ status: res.statusCode, body: b })); }).on('error', reject);
});

test('server binds 127.0.0.1 only and serves state/agent', async () => {
  const { dir, proj, sid } = tmpProject();
  const s = await startServer({ port: 0, projects: dir, pidfile: false, toast: false, root: dir });
  try {
    assert.equal(s.address.address, '127.0.0.1');
    const st = JSON.parse((await get(s.port, '/api/state')).body);
    assert.equal(st.selected, sid);
    assert.deepEqual(st.agents.map((a) => a.id).sort(), ['abc123', 'def456']);
    const d = JSON.parse((await get(s.port, '/api/agent/abc123')).body);
    assert.equal(d.events.filter((e) => e.kind === 'tool').length, 4);
    const ev = JSON.parse((await get(s.port, `/api/agent/abc123?event=${d.events.findIndex((e) => e.tool === 'Read')}`)).body);
    assert.equal(ev.event.output.length, 5000);
    assert.equal((await get(s.port, '/')).status, 200);
  } finally { s.close(); fs.rmSync(dir, { recursive: true, force: true }); }
});
