// Agent Watch — pure parsing, summarising, redaction and focus-rule logic.
// No I/O here.
import path from 'node:path';

export const DEFAULTS = { staleSec: 90, stuckSec: 240, head: 1500, tail: 500, fullMax: 20 * 1024 };

function luhn(s) {
  const d = s.replace(/\D/g, '');
  if (d.length < 13 || d.length > 16) return false;
  let sum = 0, alt = false;
  for (let i = d.length - 1; i >= 0; i--) { let n = +d[i]; if (alt) { n *= 2; if (n > 9) n -= 9; } sum += n; alt = !alt; }
  return sum % 10 === 0;
}

// A tool result that looks like a hook/guard/permission denial (policy working, not a crash).
export const isDeny = (t) => /\[[\w .-]*guard\]|blocked by (?:a )?(?:hook|guard)|permission to use .{1,60} has been denied|PreToolUse.*(?:denied|blocked)/i.test(String(t));

// ---------- redaction ----------
const LOCAL_RULES = [
  [/[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}/g, '[email]'],
  [/\b\d{3}-\d{2}-\d{4}\b/g, '[ssn]'],
  [/\b(?:\d[ -]?){13,16}\b/g, (m) => (luhn(m) ? '[card]' : m)],
  [/(X-[A-Za-z-]*(?:Token|Key|Secret)\s*:\s*)["']?[^\s"',]+/gi, '$1[redacted]'],
  [/\b([A-Z0-9_]*(?:TOKEN|SECRET|PASSWORD|PASSWD|API_KEY|CLIENT_SECRET)[A-Z0-9_]*\s*[=:]\s*)["']?[^\s"',]{6,}/g, '$1[redacted]'],
  [/\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]{12,}/gi, '$1 [redacted]'],
  [/eyJ[A-Za-z0-9_-]{8,}(?:\.[A-Za-z0-9_-]*){0,2}/g, '[jwt]'],
  [/\b[a-fA-F0-9]{32,}\b/g, '[hex-secret]'],
];
// base64-ish run >= 32 chars that mixes upper, lower and digits (avoids paths, UUIDs, slugs)
const B64_RE = /[A-Za-z0-9+_-]{32,}={0,2}/g;
function b64(m) {
  return /[A-Z]/.test(m) && /[a-z]/.test(m) && /[0-9]/.test(m) ? '[secret]' : m;
}

export function redact(text) {
  if (text == null) return '';
  let s = String(text);
  for (const [re, rep] of LOCAL_RULES) s = s.replace(re, rep);
  return s.replace(B64_RE, b64);
}

export function truncate(text, head = DEFAULTS.head, tail = DEFAULTS.tail) {
  const s = String(text ?? '');
  if (s.length <= head + tail + 80) return s;
  return `${s.slice(0, head)}\n\n… [${s.length - head - tail} chars trimmed] …\n\n${s.slice(-tail)}`;
}

function clipFull(s, max = DEFAULTS.fullMax) {
  return s.length > max ? `${s.slice(0, max)}\n… [${s.length - max} more chars not kept]` : s;
}

// Redact every string in a JSON value; cap long strings.
export function redactValue(v, cap = 4000) {
  if (typeof v === 'string') return truncate(redact(v), cap, 400);
  if (Array.isArray(v)) return v.map((x) => redactValue(x, cap));
  if (v && typeof v === 'object') {
    const o = {};
    for (const [k, x] of Object.entries(v)) o[k] = redactValue(x, cap);
    return o;
  }
  return v;
}

// ---------- summaries ----------
const base = (p) => (p ? path.basename(String(p).replace(/\\/g, '/')) : '');
const tail2 = (p) => String(p ?? '').replace(/\\/g, '/').split('/').filter(Boolean).slice(-2).join('/');
const one = (s, n = 110) => {
  const line = String(s ?? '').split(/\r?\n/).find((l) => l.trim()) ?? '';
  const t = line.trim();
  return t.length > n ? `${t.slice(0, n - 1)}…` : t;
};

export function summarize(tool, input = {}) {
  const i = input || {};
  let s;
  if (/^mcp__playwright__/.test(tool)) {
    const verb = tool.replace(/^mcp__playwright__(browser_)?/, '');
    const what = i.element || i.url || i.text || i.key || i.selector || i.filename || '';
    s = `Playwright ${verb}${what ? ` "${one(what, 80)}"` : ''}`;
  } else {
    switch (tool) {
      case 'Bash': case 'PowerShell': s = `${tool}: ${one(i.command)}`; break;
      case 'Read': {
        let r = '';
        if (i.offset || i.limit) {
          const from = Number(i.offset) || 1;
          r = i.limit ? `:${from}-${from + Number(i.limit)}` : `:${from}-`;
        }
        s = `Read: ${base(i.file_path)}${r}${i.pages ? ` p${i.pages}` : ''}`; break;
      }
      case 'Edit': case 'Write': case 'MultiEdit': s = `${tool}: ${base(i.file_path)}`; break;
      case 'NotebookEdit': s = `NotebookEdit: ${base(i.notebook_path)}`; break;
      case 'Grep': s = `Grep: "${one(i.pattern, 60)}"${i.path ? ` in ${tail2(i.path)}` : ''}${i.glob ? ` (${i.glob})` : ''}`; break;
      case 'Glob': s = `Glob: ${i.pattern}${i.path ? ` in ${tail2(i.path)}` : ''}`; break;
      case 'Agent': case 'Task': s = `Agent: ${one(i.description || i.prompt, 80)}${i.subagent_type ? ` [${i.subagent_type}]` : ''}`; break;
      case 'WebSearch': s = `WebSearch: ${one(i.query)}`; break;
      case 'WebFetch': s = `WebFetch: ${one(i.url)}`; break;
      case 'Skill': s = `Skill: ${i.skill || ''}`; break;
      case 'ToolSearch': s = `ToolSearch: ${one(i.query)}`; break;
      case 'SendMessage': s = `SendMessage: ${i.to || ''}`; break;
      case 'TodoWrite': s = `TodoWrite: ${(i.todos || []).length} items`; break;
      default: {
        const m = /^mcp__([^_]+(?:-[^_]+)*)__(.+)$/.exec(tool);
        const first = Object.values(i).find((v) => typeof v === 'string');
        s = `${m ? `${m[1]} ${m[2]}` : tool}${first ? `: ${one(first, 80)}` : ''}`;
      }
    }
  }
  return redact(s);
}

// ---------- JSONL ----------
// Splits text into complete JSON records; returns the unconsumed partial tail.
export function splitLines(text) {
  const lines = String(text).split('\n');
  const rest = lines.pop(); // '' if text ended with newline, else partial line
  const records = [];
  for (const l of lines) {
    if (!l.trim()) continue;
    try { records.push(JSON.parse(l)); } catch { /* corrupt line: skip */ }
  }
  let partial = rest ?? '';
  if (partial.trim()) {
    try { records.push(JSON.parse(partial)); partial = ''; } catch { /* still being written */ }
  }
  return { records, partial };
}

function resultText(content) {
  if (typeof content === 'string') return { text: content, images: [] };
  const texts = []; const images = [];
  for (const b of Array.isArray(content) ? content : []) {
    if (b?.type === 'text') texts.push(b.text ?? '');
    else if (b?.type === 'image') {
      const data = b.source?.data || '';
      images.push({ mediaType: b.source?.media_type || 'image/png', bytes: Math.round(data.length * 0.75), data });
    } else if (b?.type === 'tool_reference') texts.push(`[tool: ${b.tool_name}]`);
  }
  return { text: texts.join('\n'), images };
}

const ms = (t) => { const v = Date.parse(t); return Number.isFinite(v) ? v : null; };

// ---------- incremental builder ----------
export class AgentBuilder {
  constructor({ id, kind = 'agent', sessionId = null } = {}) {
    this.id = id; this.kind = kind; this.sessionId = sessionId;
    this.prompt = null; this.startedAt = null; this.lastEventAt = null;
    this.events = []; this.byToolId = new Map(); this.full = new Map(); // eventIdx -> {output, images}
    this.toolCount = 0; this.errorCount = 0; this.seenUuid = new Set();
    this.meta = {};
  }

  push(rec) {
    if (!rec || typeof rec !== 'object') return false;
    if (rec.uuid) { if (this.seenUuid.has(rec.uuid)) return false; this.seenUuid.add(rec.uuid); }
    const t = ms(rec.timestamp);
    if (rec.agentId && !this.agentId) this.agentId = rec.agentId;
    if (rec.type !== 'user' && rec.type !== 'assistant' && rec.type !== 'system') return false;
    if (t != null) { this.startedAt ??= t; this.lastEventAt = Math.max(this.lastEventAt ?? 0, t); }
    const content = rec.message?.content;

    if (rec.type === 'system') {
      if (rec.level === 'error' || rec.subtype === 'api_error') {
        this.events.push({ t, kind: 'error', summary: redact(one(rec.content || rec.error || 'error', 200)), isError: true });
        this.errorCount++;
        return true;
      }
      return false;
    }
    if (rec.type === 'user') {
      if (rec.isMeta || rec.isCompactSummary) return false;
      if (typeof content === 'string') {
        const txt = redact(content);
        if (this.prompt == null) { this.prompt = truncate(txt, 6000, 1000); return true; }
        if (/^<(local-command|command-name|system-reminder)/.test(content.trim())) return false;
        this.events.push({ t, kind: 'user', summary: one(txt, 140), text: truncate(txt, 4000, 500) });
        return true;
      }
      let changed = false;
      for (const b of Array.isArray(content) ? content : []) {
        if (b?.type === 'text') {
          if (this.prompt == null) { this.prompt = truncate(redact(b.text), 6000, 1000); changed = true; continue; }
          if (/^<(system-reminder|local-command|command-name)/.test((b.text || '').trim())) continue;
          const txt = redact(b.text);
          this.events.push({ t, kind: 'user', summary: one(txt, 140), text: truncate(txt, 4000, 500) });
          changed = true;
        } else if (b?.type === 'tool_result') {
          const { text, images } = resultText(b.content);
          const red = redact(text);
          const isError = !!b.is_error;
          const idx = this.byToolId.get(b.tool_use_id);
          const ev = idx != null ? this.events[idx] : null;
          if (isError) this.errorCount++;
          if (ev) {
            ev.output = truncate(red); ev.outputLen = red.length; ev.isError = isError; ev.done = true;
            ev.durationMs = t != null && ev.t != null ? Math.max(0, t - ev.t) : null;
            ev.deny = isDeny(red);
            if (images.length) ev.images = images.map(({ mediaType, bytes }) => ({ mediaType, bytes }));
            this.full.set(idx, { output: clipFull(red), images: images.filter((im) => im.data.length < 600_000) });
          } else {
            this.events.push({ t, kind: isError ? 'error' : 'result', summary: one(red, 140), output: truncate(red), isError, deny: isDeny(red) });
          }
          changed = true;
        }
      }
      return changed;
    }
    // assistant
    if (rec.isApiErrorMessage) {
      const txt = Array.isArray(content) ? content.map((b) => b.text || '').join(' ') : String(content ?? '');
      this.events.push({ t, kind: 'error', summary: redact(one(txt, 200)), isError: true });
      this.errorCount++;
      return true;
    }
    let changed = false;
    for (const b of Array.isArray(content) ? content : []) {
      if (b?.type === 'text' && b.text?.trim()) {
        this.events.push({ t, kind: 'text', summary: one(redact(b.text), 140), text: truncate(redact(b.text), 12000, 2000) });
        changed = true;
      } else if (b?.type === 'thinking' || b?.type === 'redacted_thinking') {
        const last = this.events[this.events.length - 1];
        if (last?.kind === 'thinking') { last.t2 = t; continue; }
        this.events.push({ t, kind: 'thinking', summary: 'thinking…' });
        changed = true;
      } else if (b?.type === 'tool_use') {
        const input = redactValue(b.input ?? {});
        this.byToolId.set(b.id, this.events.length);
        this.events.push({ t, kind: 'tool', tool: b.name, id: b.id, summary: summarize(b.name, b.input ?? {}), input, done: false, rawPathHints: pathHints(b.input) });
        this.toolCount++;
        changed = true;
      }
    }
    return changed;
  }

  snapshot({ now = Date.now(), staleSec = DEFAULTS.staleSec, stuckSec = DEFAULTS.stuckSec, rules = [] } = {}) {
    const m = this.meta || {};
    const status = computeStatus(this, m, now, { staleSec, stuckSec });
    const focus = evalRules(this, rules, status);
    const finalText = [...this.events].reverse().find((e) => e.kind === 'text');
    return {
      id: this.id, kind: this.kind, sessionId: this.sessionId,
      type: this.kind === 'main' ? 'main session' : (m.agentType || 'agent'),
      description: m.description || (this.kind === 'main' ? 'Main session' : ''),
      prompt: this.prompt ?? '', status, finished: !!m.finished,
      startedAt: this.startedAt, lastEventAt: this.lastEventAt,
      toolCount: this.toolCount, errorCount: this.errorCount, eventCount: this.events.length,
      lastSummary: this.events.length ? this.events[this.events.length - 1].summary : '',
      finalText: finalText ? finalText.text : '',
      spawnDepth: m.spawnDepth ?? null, toolUseId: m.toolUseId ?? null, focus,
    };
  }

  // Full detail (events without per-event full output; strip internal fields)
  detail(opts) {
    return { ...this.snapshot(opts), events: this.events.map(publicEvent) };
  }
}

function publicEvent(e) {
  const { rawPathHints, ...rest } = e;
  return rest;
}

function pathHints(input) {
  if (!input || typeof input !== 'object') return '';
  return ['file_path', 'path', 'notebook_path', 'pattern', 'command', 'url', 'glob']
    .map((k) => (typeof input[k] === 'string' ? input[k] : '')).filter(Boolean).join('\n').replace(/\\/g, '/');
}

export function computeStatus(b, meta, now, { staleSec = DEFAULTS.staleSec, stuckSec = DEFAULTS.stuckSec } = {}) {
  if (meta?.finished) return 'done';
  const age = b.lastEventAt != null ? (now - b.lastEventAt) / 1000 : 0;
  if (age > stuckSec) return 'stuck';
  if (age > staleSec) return 'stale';
  // running = an unanswered tool_use among the most recent events (parallel calls answer out of order)
  for (let i = b.events.length - 1; i >= Math.max(0, b.events.length - 20); i--) {
    const e = b.events[i];
    if (e.kind === 'tool' && !e.done) return 'running';
    if (e.kind === 'text' || e.kind === 'user') break;
  }
  return 'thinking';
}

// ---------- focus rules ----------
export function compileRules(list) {
  return (Array.isArray(list) ? list : []).map((r) => {
    const m = r.match || {};
    return {
      name: r.name, action: r.action || 'highlight',
      tool: m.tool ? new RegExp(m.tool, 'i') : null,
      path: m.path ? new RegExp(m.path, 'i') : null,
      isError: typeof m.isError === 'boolean' ? m.isError : null,
      deny: typeof m.deny === 'boolean' ? m.deny : null,
      stuck: m.stuck === true,
    };
  });
}

export function evalRules(b, rules, status) {
  const hits = [];
  if (!rules?.length) return hits;
  b.events.forEach((e, idx) => {
    if (e.kind !== 'tool' && e.kind !== 'result' && e.kind !== 'error') return;
    for (const r of rules) {
      if (r.stuck) continue;
      if (r.tool && !(e.tool && r.tool.test(e.tool))) continue;
      if (r.path && !r.path.test(e.rawPathHints || '')) continue;
      if (r.isError != null && !!e.isError !== r.isError) continue;
      if (r.deny != null && !!e.deny !== r.deny) continue;
      if (!r.tool && !r.path && r.isError == null && r.deny == null) continue;
      hits.push({ rule: r.name, action: r.action, idx, t: e.t, summary: e.summary });
    }
  });
  if (status === 'stuck') for (const r of rules) if (r.stuck) hits.push({ rule: r.name, action: r.action, idx: -1, t: b.lastEventAt, summary: 'no activity' });
  return hits;
}

// ---------- one-shot API ----------
export function parseAgentFile(text, meta = {}, opts = {}) {
  const id = opts.id || meta?.agentId || null;
  const b = new AgentBuilder({ id, kind: opts.kind || 'agent', sessionId: opts.sessionId || null });
  b.meta = meta || {};
  const { records } = splitLines(text);
  for (const r of records) b.push(r);
  if (!b.id) b.id = b.agentId || 'unknown';
  const rules = opts.rules ? (opts.rules[0]?.name !== undefined && opts.rules[0]?.match ? compileRules(opts.rules) : opts.rules) : [];
  return b.detail({ ...opts, rules });
}

export function projectDirName(p) {
  return String(p).replace(/[^a-zA-Z0-9]/g, '-');
}
