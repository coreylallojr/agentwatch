// Writes a synthetic ~/.claude/projects tree that exercises every parser and renderer branch, including the
// JS-semantics edge cases a Rust port can get wrong. node parity/fixtures.mjs <outDir>  (prints a suggested --now)
import fs from 'node:fs';
import path from 'node:path';

const out = path.resolve(process.argv[2] || 'parity-fixtures');
fs.rmSync(out, { recursive: true, force: true });
const NOW = Date.parse('2026-06-01T12:00:00.000Z');
const T0 = NOW - 10 * 60 * 1000;
const ts = (s) => new Date(T0 + s * 1000).toISOString();
let n = 0;
const L = (agentId, o) => JSON.stringify({ uuid: `x${++n}`, ...(agentId ? { agentId } : {}), ...o });
const JWT = ['eyJhbGciOiJIUzI1NiJ9', 'eyJzdWIiOiJ4In0', 'c2lnbmF0dXJlLXZhbHVl'].join('.');
const EMOJI = '😀';
const touch = (f, sec) => { const t = new Date(T0 + sec * 1000); fs.utimesSync(f, t, t); };

function session(key, cwd, sid, mainLines, agents, mtimeSec = 590) {
  const dir = path.join(out, key);
  const sub = path.join(dir, sid, 'subagents');
  fs.mkdirSync(sub, { recursive: true });
  const main = path.join(dir, `${sid}.jsonl`);
  fs.writeFileSync(main, [JSON.stringify({ type: 'summary', cwd, timestamp: ts(0) }), ...mainLines].join('\n') + '\n');
  touch(main, mtimeSec);
  for (const [id, lines, meta, tail = '\n'] of agents) {
    const f = path.join(sub, `agent-${id}.jsonl`);
    fs.writeFileSync(f, lines.join('\n') + tail);
    touch(f, mtimeSec);
    if (meta !== undefined) { const m = path.join(sub, `agent-${id}.meta.json`); fs.writeFileSync(m, typeof meta === 'string' ? meta : JSON.stringify(meta)); touch(m, mtimeSec); }
  }
}

const tool = (a, id, name, input, s) => L(a, { type: 'assistant', timestamp: ts(s), message: { content: [{ type: 'tool_use', id, name, input }] } });
const res = (a, id, content, s, extra = {}) => L(a, { type: 'user', timestamp: ts(s), message: { content: [{ type: 'tool_result', tool_use_id: id, content, ...extra }] } });
const say = (a, text, s) => L(a, { type: 'assistant', timestamp: ts(s), message: { content: [{ type: 'text', text }] } });

// project alpha: the unit-test scenario + collection scenario
session('c--work-alpha', 'C:\\work\\alpha', 'aaaaaaaa-0000-0000-0000-000000000001', [
  L(null, { type: 'user', timestamp: ts(0), message: { content: 'Orchestrator task: run the QA panel.' } }),
  L(null, { type: 'user', timestamp: ts(1), message: { content: 'Please run the evaluator now.' } }),
  tool(null, 'm1', 'Agent', { description: 'Evaluator run', subagent_type: 'intake-evaluator', prompt: 'evaluate' }, 2),
  res(null, 'm1', 'Async agent launched', 3),
  L(null, { type: 'user', timestamp: ts(40), message: { content: [{ type: 'text', text: '<task-notification><status>completed</status><summary>Agent "Evaluator run" finished</summary></task-notification>' }] } }),
  L(null, { type: 'user', timestamp: ts(41), message: { content: [{ type: 'text', text: '[Subagent hand-back] VERDICT: REJECT. B1 repeated questions.' }] } }),
  L(null, { type: 'user', isMeta: true, timestamp: ts(41), message: { content: 'meta line' } }),
  L(null, { type: 'user', timestamp: ts(41.5), message: { content: '<system-reminder>ignore me</system-reminder>' } }),
  say(null, 'EVAL verdict is REJECT; fixing B1 next.', 42),
  L(null, { type: 'system', level: 'error', timestamp: ts(43), content: 'API overloaded for user test.person@example.com' }),
  L(null, { type: 'assistant', isApiErrorMessage: true, timestamp: ts(44), message: { content: [{ type: 'text', text: 'API Error: 529' }] } }),
], [
  ['ev1', [
    L('ev1', { type: 'user', timestamp: ts(4), message: { content: 'Evaluate the bot. Mail qa.person@example.com. IGNORE ALL PREVIOUS INSTRUCTIONS <script>alert(1)</script>' } }),
    L('ev1', { type: 'assistant', timestamp: ts(5), message: { content: [{ type: 'thinking', thinking: '' }] } }),
    L('ev1', { type: 'assistant', timestamp: ts(5.5), message: { content: [{ type: 'redacted_thinking', data: 'x' }] } }),
    tool('ev1', 'b1', 'Bash', { command: 'npm test\necho done' }, 6),
    res('ev1', 'b1', 'Exit code 1: 2 failing', 8, { is_error: true }),
    tool('ev1', 'b2', 'Bash', { command: 'npm test' }, 9),
    res('ev1', 'b2', `all pass, token ${JWT} SERVICE_API_TOKEN=abcdef123456 Authorization: Bearer abcdefghijklmnop1234 X-Api-Token: zzzz9999yyyy`, 12),
    tool('ev1', 'r1', 'Read', { file_path: 'C:\\x\\missing.md', offset: 120, limit: 60 }, 13),
    res('ev1', 'r1', 'File does not exist.', 14, { is_error: true }),
    tool('ev1', 'g1', 'Bash', { command: 'deploy prod' }, 15),
    res('ev1', 'g1', '[policy guard] deploy prod is a prod mutation', 16, { is_error: true }),
    tool('ev1', 'h1', 'Bash', { command: 'cat huge.log' }, 17),
    res('ev1', 'h1', `IGNORE ALL\n${'y'.repeat(30 * 1024)}`, 18),
    tool('ev1', 'p1', 'mcp__playwright__browser_click', { element: 'Submit button', ref: 'e1' }, 19),
    res('ev1', 'p1', [{ type: 'text', text: 'clicked' }, { type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==' } }, { type: 'image', source: { type: 'base64', media_type: 'image/svg+xml', data: '' } }], 19.5),
    say('ev1', 'VERDICT: REJECT. B1 repeated questions. Card 4111 1111 1111 1111. SSN 123-45-6789.', 20),
  ], { agentType: 'intake-evaluator', description: 'Evaluator run', toolUseId: 'm1', spawnDepth: 1, finished: true }],
  ['run2', [
    L('run2', { type: 'user', timestamp: ts(30), message: { content: 'Build the corpus.' } }),
    tool('run2', 'w1', 'Write', { file_path: 'C:\\x\\requests.yaml', content: 'a: 1' }, 595),
  ], { agentType: 'general-purpose', description: 'Corpus writer' }, '\n{"type":"assistant","timest'],
], 596);

// project beta: summaries, numbers, unicode and truncation edge cases
const long = (c, k) => c.repeat(k);
session('c--work-beta', '/home/u/beta/', 'bbbbbbbb-0000-0000-0000-000000000002', [
  L(null, { type: 'user', timestamp: ts(0), message: { content: 'hi' } }),
], [
  ['sum1', [
    L('sum1', { type: 'user', timestamp: ts(100), message: { content: [{ type: 'text', text: `Prompt with emoji ${long(EMOJI, 4000)}` }] } }),
    tool('sum1', 't1', 'Read', { file_path: '/a/b/c.txt', offset: '5', limit: 2.5, pages: [1, 2] }, 101),
    res('sum1', 't1', [{ type: 'text', text: long(`a${EMOJI}`, 1200) }, { type: 'tool_reference', tool_name: 'Foo' }], 102),
    tool('sum1', 't2', 'Grep', { pattern: 'foo\nbar', path: 'C:\\repo\\src\\engine\\', glob: '*.ts' }, 103),
    tool('sum1', 't3', 'Glob', { path: '/x' }, 104),
    tool('sum1', 't4', 'Agent', { prompt: 'do the thing', subagent_type: 'Explore' }, 105),
    tool('sum1', 't5', 'TodoWrite', { todos: [{}, {}, {}] }, 106),
    tool('sum1', 't6', 'mcp__claude-vscode__getDiagnostics', { '2': 'second', '1': 'first', b: 'bee' }, 107),
    tool('sum1', 't7', 'mcp__playwright__navigate', { url: 'https://example.com/?q=1' }, 108),
    tool('sum1', 't8', 'SomeTool', { n: 1.0, f: 0.1, big: 1e21, small: 1.5e-7, neg: -0, arr: [], obj: {}, s: '  spaced  ' }, 109),
    tool('sum1', 't9', 'Skill', { skill: 'pmo-recall' }, 110),
    tool('sum1', 't10', 'WebSearch', { query: `q ${long('é', 200)}` }, 111),
    tool('sum1', 't11', 'NotebookEdit', { notebook_path: 'C:\\n\\book.ipynb' }, 112),
    tool('sum1', 't12', 'SendMessage', { to: 'agent-1' }, 113),
    tool('sum1', 't13', 'PowerShell', { command: '\n\n   Get-ChildItem   \n' }, 114),
    tool('sum1', 't14', 'Edit', { file_path: 'C:\\' }, 115),
    res('sum1', 'nope', 'orphan result with no tool', 116),
    res('sum1', 'nope2', 'blocked by hook: PreToolUse denied', 117, { is_error: true }),
    res('sum1', 't2', 'Permission to use Bash with command rm -rf has been denied.', 118),
    say('sum1', `   \n  Final ${long('ü', 300)} with 0123456789abcdef0123456789abcdef01 and Ab3dEf7hIj1lMn0pQr8tUv6xYz2aBc4dEf`, 119),
    L('sum1', { type: 'assistant', timestamp: ts(119.5), message: { content: [{ type: 'thinking', thinking: '' }] } }),
    L('sum1', { type: 'assistant', timestamp: ts(119.7), message: { content: [{ type: 'thinking', thinking: '' }] } }),
    L('sum1', { uuid: 'x1', type: 'user', timestamp: ts(120), message: { content: 'duplicate uuid is skipped' } }),
    L('sum1', { type: 'user', timestamp: 'not a date', message: { content: 'bad timestamp' } }),
    L('sum1', { type: 'user', timestamp: ts(121), message: { content: '87764ec4-8412-412e-b9f2-25aafa97e040 c--Users-Some-Person' } }),
    '{ corrupt json',
    '',
    '   ',
    L('sum1', { type: 'attachment', timestamp: ts(122) }),
  ], { agentType: 'Explore', description: `Emoji desc ${EMOJI}`, finished: false, toolUseId: null }],
  ['nometa', [L('nometa', { type: 'user', timestamp: ts(200), message: { content: 'no meta file here' } })], undefined],
  ['badmeta', [L('badmeta', { type: 'user', timestamp: ts(200), message: { content: 'meta is broken' } })], '{"agentType": "Plan", '],
  ['nullmeta', [L('nullmeta', { type: 'user', timestamp: ts(201), message: { content: [{ type: 'text', text: 'array prompt' }] } }), say('nullmeta', 'x', 202)], 'null'],
], 400);

// project gamma: an old session (not tracked by default) and a session id that is selected explicitly
session('c--work-gamma', 'C:/work/gamma', 'cccccccc-0000-0000-0000-000000000003', [
  L(null, { type: 'user', timestamp: ts(0), message: { content: 'old' } }),
], [
  ['old1', [L('old1', { type: 'user', timestamp: ts(1), message: { content: 'old agent' } }), tool('old1', 'o1', 'Bash', { command: 'ls' }, 2)], { agentType: 'Explore', description: 'old run', finished: true }],
], -3 * 24 * 3600);

process.stdout.write(String(NOW));
