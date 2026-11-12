//! Port of src/parse.mjs: redaction, summaries, the incremental transcript builder, status and focus rules.
use crate::js::{self, len16, prop, truthy, RE_S};
use regex::{Captures, Regex, RegexBuilder};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub const STALE_SEC: f64 = 90.0;
pub const STUCK_SEC: f64 = 240.0;
pub const HEAD: usize = 1500;
pub const TAIL: usize = 500;
pub const FULL_MAX: usize = 20 * 1024;

fn re(p: &str) -> Regex { Regex::new(p).unwrap_or_else(|e| panic!("bad built-in regex {p}: {e}")) }

// ---------------------------------------------------------------- redaction

fn luhn(s: &str) -> bool {
    let d: Vec<u32> = s.chars().filter_map(|c| c.to_digit(10)).collect();
    if d.len() < 13 || d.len() > 16 { return false; }
    let (mut sum, mut alt) = (0, false);
    for &n in d.iter().rev() {
        let mut n = n;
        if alt { n *= 2; if n > 9 { n -= 9; } }
        sum += n; alt = !alt;
    }
    sum % 10 == 0
}

static NOT_LT: &str = r"[^\n\r\x{2028}\x{2029}]";

static DENY: LazyLock<Regex> = LazyLock::new(|| re(&format!(
    r"(?i-u:\[[a-z0-9_ .-]*guard\])|(?i-u:blocked by (?:a )?(?:hook|guard))|(?i-u:permission to use ){NOT_LT}{{1,60}}(?i-u: has been denied)|(?i-u:pretooluse){NOT_LT}*(?i-u:denied|blocked)"
)));

/// A tool result that looks like a hook/guard/permission denial (policy working, not a crash).
pub fn is_deny(t: &str) -> bool { DENY.is_match(t) }

struct Rules {
    email: Regex, ssn: Regex, card: Regex, xhdr: Regex, envtok: Regex, bearer: Regex, jwt: Regex, hex: Regex, b64: Regex,
}

static R: LazyLock<Rules> = LazyLock::new(|| Rules {
    email: re(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}"),
    ssn: re(r"(?-u:\b)[0-9]{3}-[0-9]{2}-[0-9]{4}(?-u:\b)"),
    card: re(r"(?-u:\b)(?:[0-9][ -]?){13,16}(?-u:\b)"),
    xhdr: re(&format!(r#"((?i-u:x-[a-z-]*(?:token|key|secret))[{RE_S}]*:[{RE_S}]*)["']?[^{RE_S}"',]+"#)),
    envtok: re(&format!(r#"(?-u:\b)([A-Z0-9_]*(?:TOKEN|SECRET|PASSWORD|PASSWD|API_KEY|CLIENT_SECRET)[A-Z0-9_]*[{RE_S}]*[=:][{RE_S}]*)["']?[^{RE_S}"',]{{6,}}"#)),
    bearer: re(&format!(r"(?-u:\b)((?i-u:bearer|basic))[{RE_S}]+[A-Za-z0-9._~+/=-]{{12,}}")),
    jwt: re(r"eyJ[A-Za-z0-9_-]{8,}(?:\.[A-Za-z0-9_-]*){0,2}"),
    hex: re(r"(?-u:\b)[a-fA-F0-9]{32,}(?-u:\b)"),
    b64: re(r"[A-Za-z0-9+_-]{32,}={0,2}"),
});

/// All nine rules in one automaton: a single pass tells which rules can match at all. Rules apply in order, and a
/// replacement can create a new `\b` boundary for a later rule (e.g. `[email]` before a hex run), so the set is
/// re-run on the new text after every rule that actually changed it.
static SET: LazyLock<regex::RegexSet> = LazyLock::new(|| {
    let r = &*R;
    regex::RegexSet::new([&r.email, &r.ssn, &r.card, &r.xhdr, &r.envtok, &r.bearer, &r.jwt, &r.hex, &r.b64].map(|x| x.as_str())).expect("redaction set")
});

fn apply(i: usize, s: &str) -> std::borrow::Cow<'_, str> {
    let r = &*R;
    match i {
        0 => r.email.replace_all(s, "[email]"),
        1 => r.ssn.replace_all(s, "[ssn]"),
        2 => r.card.replace_all(s, |c: &Captures| if luhn(&c[0]) { "[card]".to_string() } else { c[0].to_string() }),
        3 => r.xhdr.replace_all(s, "${1}[redacted]"),
        4 => r.envtok.replace_all(s, "${1}[redacted]"),
        5 => r.bearer.replace_all(s, "${1} [redacted]"),
        6 => r.jwt.replace_all(s, "[jwt]"),
        7 => r.hex.replace_all(s, "[hex-secret]"),
        _ => r.b64.replace_all(s, |c: &Captures| {
            let m = &c[0];
            let (u, l, d) = (m.bytes().any(|b| b.is_ascii_uppercase()), m.bytes().any(|b| b.is_ascii_lowercase()), m.bytes().any(|b| b.is_ascii_digit()));
            if u && l && d { "[secret]".to_string() } else { m.to_string() }
        }),
    }
}

pub fn redact(text: &str) -> String {
    let mut hits = SET.matches(text);
    if !hits.matched_any() { return text.to_string(); }
    let mut buf: Option<String> = None;
    for i in 0..9 {
        if !hits.matched(i) { continue; }
        let cur = buf.as_deref().unwrap_or(text);
        let changed = match apply(i, cur) { std::borrow::Cow::Owned(o) if o != cur => Some(o), _ => None };
        if let Some(o) = changed { hits = SET.matches(&o); buf = Some(o); }
    }
    buf.unwrap_or_else(|| text.to_string())
}

/// JS `redact(v)` for an arbitrary value (`null`/`undefined` -> '').
pub fn redact_val(v: Option<&Value>) -> String {
    match v { None | Some(Value::Null) => String::new(), Some(v) => redact(&js::string(Some(v))) }
}

pub fn truncate(s: &str, head: usize, tail: usize) -> String {
    let l = len16(s);
    if l <= head + tail + 80 { return s.to_string(); }
    format!("{}\n\n… [{} chars trimmed] …\n\n{}", js::head16(s, head), l - head - tail, js::tail16(s, tail))
}

fn clip_full(s: &str) -> String {
    let l = len16(s);
    if l > FULL_MAX { format!("{}\n… [{} more chars not kept]", js::head16(s, FULL_MAX), l - FULL_MAX) } else { s.to_string() }
}

/// Redact every string in a JSON value; cap long strings.
pub fn redact_value(v: &Value, cap: usize) -> Value {
    match v {
        Value::String(s) => Value::String(truncate(&redact(s), cap, 400)),
        Value::Array(a) => Value::Array(a.iter().map(|x| redact_value(x, cap)).collect()),
        Value::Object(m) => {
            let mut o = Map::new();
            for k in js::js_keys(m) { o.insert(k.clone(), redact_value(&m[k.as_str()], cap)); }
            Value::Object(o)
        }
        v => v.clone(),
    }
}

// ---------------------------------------------------------------- summaries

/// Node `path.basename` (win32 flavour on Windows, posix elsewhere).
pub fn basename(p: &str) -> String {
    let win = cfg!(windows);
    let sep = |c: char| c == '/' || (win && c == '\\');
    let mut s = p;
    if win {
        let b = s.as_bytes();
        if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' { s = &s[2..]; }
    }
    let s = s.trim_end_matches(sep);
    match s.rfind(sep) { Some(i) => s[i + 1..].to_string(), None => s.to_string() }
}

fn base(p: Option<&Value>) -> String {
    if !truthy(p) { return String::new(); }
    basename(&js::string(p).replace('\\', "/"))
}

fn tail2(p: Option<&Value>) -> String {
    let s = js::string_or_empty(p).replace('\\', "/");
    let parts: Vec<&str> = s.split('/').filter(|x| !x.is_empty()).collect();
    parts[parts.len().saturating_sub(2)..].join("/")
}

pub fn one_str(s: &str, n: usize) -> String {
    let line = s.split('\n').find(|l| !js::trim(l).is_empty()).unwrap_or("");
    let t = js::trim(line);
    if len16(t) > n { format!("{}…", js::head16(t, n - 1)) } else { t.to_string() }
}

pub fn one(v: Option<&Value>, n: usize) -> String { one_str(&js::string_or_empty(v), n) }

static PW: LazyLock<Regex> = LazyLock::new(|| re(r"^mcp__playwright__(?:browser_)?"));
static MCP: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"^mcp__([^_]+(?:-[^_]+)*)__({NOT_LT}+)$")));

/// JS `Object.values(i).find(v => typeof v === 'string')`.
fn first_string(i: &Value) -> Option<String> {
    match i {
        Value::Object(m) => js::js_keys(m).into_iter().find_map(|k| m[k.as_str()].as_str().map(str::to_string)),
        Value::Array(a) => a.iter().find_map(|v| v.as_str().map(str::to_string)),
        Value::String(s) => s.chars().next().map(|c| c.to_string()),
        _ => None,
    }
}

pub fn summarize(tool: &str, input: Option<&Value>) -> String {
    let empty = Value::Object(Map::new());
    let i: &Value = if truthy(input) { input.unwrap() } else { &empty };
    let g = |k: &str| prop(Some(i), k);
    let s = if tool.starts_with("mcp__playwright__") {
        let verb = PW.replace(tool, "");
        let what = [g("element"), g("url"), g("text"), g("key"), g("selector"), g("filename")].into_iter().find(|v| truthy(*v)).flatten();
        format!("Playwright {verb}{}", if truthy(what) { format!(" \"{}\"", one(what, 80)) } else { String::new() })
    } else {
        match tool {
            "Bash" | "PowerShell" => format!("{tool}: {}", one(g("command"), 110)),
            "Read" => {
                let mut r = String::new();
                if truthy(g("offset")) || truthy(g("limit")) {
                    let o = js::number(g("offset"));
                    let from = if o != 0.0 && !o.is_nan() { o } else { 1.0 };
                    r = if truthy(g("limit")) { format!(":{}-{}", js::num(from), js::num(from + js::number(g("limit")))) } else { format!(":{}-", js::num(from)) };
                }
                let pages = if truthy(g("pages")) { format!(" p{}", js::string(g("pages"))) } else { String::new() };
                format!("Read: {}{r}{pages}", base(g("file_path")))
            }
            "Edit" | "Write" | "MultiEdit" => format!("{tool}: {}", base(g("file_path"))),
            "NotebookEdit" => format!("NotebookEdit: {}", base(g("notebook_path"))),
            "Grep" => format!("Grep: \"{}\"{}{}", one(g("pattern"), 60),
                if truthy(g("path")) { format!(" in {}", tail2(g("path"))) } else { String::new() },
                if truthy(g("glob")) { format!(" ({})", js::string(g("glob"))) } else { String::new() }),
            "Glob" => format!("Glob: {}{}", js::string(g("pattern")), if truthy(g("path")) { format!(" in {}", tail2(g("path"))) } else { String::new() }),
            "Agent" | "Task" => format!("Agent: {}{}", one(js::or(g("description"), g("prompt")), 80),
                if truthy(g("subagent_type")) { format!(" [{}]", js::string(g("subagent_type"))) } else { String::new() }),
            "WebSearch" => format!("WebSearch: {}", one(g("query"), 110)),
            "WebFetch" => format!("WebFetch: {}", one(g("url"), 110)),
            "Skill" => format!("Skill: {}", if truthy(g("skill")) { js::string(g("skill")) } else { String::new() }),
            "ToolSearch" => format!("ToolSearch: {}", one(g("query"), 110)),
            "SendMessage" => format!("SendMessage: {}", if truthy(g("to")) { js::string(g("to")) } else { String::new() }),
            "TodoWrite" => {
                let n = match g("todos") {
                    v if !truthy(v) => "0".to_string(),
                    Some(Value::Array(a)) => a.len().to_string(),
                    Some(Value::String(s)) => len16(s).to_string(),
                    _ => "undefined".to_string(),
                };
                format!("TodoWrite: {n} items")
            }
            _ => {
                let head = match MCP.captures(tool) { Some(c) => format!("{} {}", &c[1], &c[2]), None => tool.to_string() };
                match first_string(i) { Some(f) if !f.is_empty() => format!("{head}: {}", one_str(&f, 80)), _ => head }
            }
        }
    };
    redact(&s)
}

// ---------------------------------------------------------------- JSONL

pub struct Split { pub records: Vec<Value>, pub partial: String }

/// Splits text into complete JSON records; returns the unconsumed partial tail.
pub fn split_lines(text: &str) -> Split {
    let mut lines: Vec<&str> = text.split('\n').collect();
    let rest = lines.pop().unwrap_or("");
    let mut records = Vec::new();
    for l in lines {
        if js::trim(l).is_empty() { continue; }
        if let Some(v) = js::parse_line(l) { records.push(v); }
    }
    let mut partial = rest.to_string();
    if !js::trim(&partial).is_empty() {
        if let Some(v) = js::parse_line(&partial) { records.push(v); partial.clear(); }
    }
    Split { records, partial }
}

#[derive(Clone, Debug)]
pub struct Image { pub media_type: String, pub bytes: f64, pub data: String }

fn result_text(content: Option<&Value>) -> (String, Vec<Image>) {
    if let Some(Value::String(s)) = content { return (s.clone(), vec![]); }
    let mut texts: Vec<String> = vec![];
    let mut images = vec![];
    if let Some(Value::Array(a)) = content {
        for b in a {
            let ty = prop(Some(b), "type").and_then(Value::as_str);
            match ty {
                Some("text") => texts.push(js::string_or_empty(prop(Some(b), "text"))),
                Some("image") => {
                    let src = prop(Some(b), "source");
                    let data = prop(src, "data").and_then(Value::as_str).unwrap_or("").to_string();
                    let mt = prop(src, "media_type");
                    let media_type = if truthy(mt) { js::string(mt) } else { "image/png".into() };
                    images.push(Image { media_type, bytes: js::round(len16(&data) as f64 * 0.75), data });
                }
                Some("tool_reference") => texts.push(format!("[tool: {}]", js::string(prop(Some(b), "tool_name")))),
                _ => {}
            }
        }
    }
    (texts.join("\n"), images)
}

// ---------------------------------------------------------------- events

#[derive(Clone, Debug, Default)]
pub struct Event {
    pub t: Option<f64>,
    pub kind: &'static str,
    pub tool: Option<Value>,
    pub id: Option<Value>,
    pub summary: String,
    pub text: Option<String>,
    pub input: Option<Value>,
    pub done: Option<bool>,
    pub t2: Option<Option<f64>>,
    pub output: Option<String>,
    pub output_len: Option<usize>,
    pub is_error: Option<bool>,
    pub duration_ms: Option<Option<f64>>,
    pub deny: Option<bool>,
    /// summary images (no data) on the event itself
    pub images: Option<Vec<Image>>,
    pub raw_path_hints: String,
}

impl Event {
    pub fn tool_str(&self) -> String { match &self.tool { None => String::new(), Some(v) => js::string_or_empty(Some(v)) } }
    pub fn is_err(&self) -> bool { self.is_error.unwrap_or(false) }
    pub fn is_done(&self) -> bool { self.done.unwrap_or(false) }
    pub fn is_deny(&self) -> bool { self.deny.unwrap_or(false) }

    /// Public JSON (rawPathHints stripped). `images_with_data` controls whether image data is included.
    pub fn to_value(&self, images_with_data: bool) -> Value {
        let mut m = Map::new();
        let f = |x: Option<f64>| x.map(|v| Value::from(v)).unwrap_or(Value::Null);
        m.insert("t".into(), f(self.t));
        m.insert("kind".into(), Value::from(self.kind));
        if let Some(v) = &self.tool { m.insert("tool".into(), v.clone()); }
        if let Some(v) = &self.id { m.insert("id".into(), v.clone()); }
        m.insert("summary".into(), Value::from(self.summary.as_str()));
        if let Some(v) = &self.text { m.insert("text".into(), Value::from(v.as_str())); }
        if let Some(v) = &self.input { m.insert("input".into(), v.clone()); }
        if let Some(v) = self.done { m.insert("done".into(), Value::from(v)); }
        if let Some(v) = self.t2 { m.insert("t2".into(), f(v)); }
        if let Some(v) = &self.output { m.insert("output".into(), Value::from(v.as_str())); }
        if let Some(v) = self.output_len { m.insert("outputLen".into(), Value::from(v as f64)); }
        if let Some(v) = self.is_error { m.insert("isError".into(), Value::from(v)); }
        if let Some(v) = self.duration_ms { m.insert("durationMs".into(), f(v)); }
        if let Some(v) = self.deny { m.insert("deny".into(), Value::from(v)); }
        if let Some(v) = &self.images { m.insert("images".into(), images_value(v, images_with_data)); }
        Value::Object(m)
    }
}

pub fn images_value(v: &[Image], with_data: bool) -> Value {
    Value::Array(v.iter().map(|im| {
        let mut o = Map::new();
        o.insert("mediaType".into(), Value::from(im.media_type.as_str()));
        o.insert("bytes".into(), Value::from(im.bytes));
        if with_data { o.insert("data".into(), Value::from(im.data.as_str())); }
        Value::Object(o)
    }).collect())
}

#[derive(Clone, Debug)]
pub struct Full { pub output: String, pub images: Vec<Image> }

fn path_hints(input: Option<&Value>) -> String {
    let Some(Value::Object(m)) = input else { return String::new() };
    ["file_path", "path", "notebook_path", "pattern", "command", "url", "glob"].iter()
        .filter_map(|k| m.get(*k).and_then(Value::as_str)).filter(|s| !s.is_empty())
        .collect::<Vec<_>>().join("\n").replace('\\', "/")
}

static SKIP_STR: LazyLock<Regex> = LazyLock::new(|| re(r"^<(?:local-command|command-name|system-reminder)"));

fn key(v: Option<&Value>) -> String { match v { None => "u".into(), Some(v) => format!("v{}", js::json(v)) } }

// ---------------------------------------------------------------- builder

#[derive(Clone, Debug)]
pub struct AgentBuilder {
    pub id: String,
    pub kind: &'static str,
    pub session_id: Option<String>,
    pub prompt: Option<String>,
    pub started_at: Option<f64>,
    pub last_event_at: Option<f64>,
    pub events: Vec<Event>,
    by_tool_id: HashMap<String, usize>,
    pub full: HashMap<usize, Full>,
    pub tool_count: usize,
    pub error_count: usize,
    seen_uuid: HashSet<String>,
    pub meta: Value,
    pub agent_id: Option<Value>,
}

pub struct Opts<'a> { pub now: f64, pub stale_sec: f64, pub stuck_sec: f64, pub rules: &'a [Rule] }

impl AgentBuilder {
    pub fn new(id: &str, kind: &'static str, session_id: Option<String>) -> Self {
        AgentBuilder {
            id: id.to_string(), kind, session_id, prompt: None, started_at: None, last_event_at: None, events: vec![],
            by_tool_id: HashMap::new(), full: HashMap::new(), tool_count: 0, error_count: 0, seen_uuid: HashSet::new(),
            meta: Value::Object(Map::new()), agent_id: None,
        }
    }

    pub fn push(&mut self, rec: &Value) -> bool {
        if !(rec.is_object() || rec.is_array()) { return false; }
        let r = Some(rec);
        let uuid = prop(r, "uuid");
        if truthy(uuid) {
            let k = key(uuid);
            if self.seen_uuid.contains(&k) { return false; }
            self.seen_uuid.insert(k);
        }
        let t = match prop(r, "timestamp") { Some(Value::String(s)) => js::date_parse(s), _ => None };
        let aid = prop(r, "agentId");
        if truthy(aid) && !truthy(self.agent_id.as_ref()) { self.agent_id = aid.cloned(); }
        let ty = prop(r, "type").and_then(Value::as_str).unwrap_or("");
        if ty != "user" && ty != "assistant" && ty != "system" { return false; }
        if let Some(t) = t {
            if self.started_at.is_none() { self.started_at = Some(t); }
            self.last_event_at = Some(self.last_event_at.unwrap_or(0.0).max(t));
        }
        let content = prop(prop(r, "message"), "content");

        if ty == "system" {
            let lvl = prop(r, "level").and_then(Value::as_str) == Some("error");
            let api = prop(r, "subtype").and_then(Value::as_str) == Some("api_error");
            if lvl || api {
                let src = js::or(js::or(prop(r, "content"), prop(r, "error")), None);
                let summary = if truthy(src) { redact(&one(src, 200)) } else { redact(&one_str("error", 200)) };
                self.events.push(Event { t, kind: "error", summary, is_error: Some(true), ..Default::default() });
                self.error_count += 1;
                return true;
            }
            return false;
        }
        if ty == "user" {
            if truthy(prop(r, "isMeta")) || truthy(prop(r, "isCompactSummary")) { return false; }
            if let Some(Value::String(c)) = content {
                let txt = redact(c);
                if self.prompt.is_none() { self.prompt = Some(truncate(&txt, 6000, 1000)); return true; }
                if SKIP_STR.is_match(js::trim(c)) { return false; }
                self.events.push(Event { t, kind: "user", summary: one_str(&txt, 140), text: Some(truncate(&txt, 4000, 500)), ..Default::default() });
                return true;
            }
            let mut changed = false;
            if let Some(Value::Array(blocks)) = content {
                for b in blocks {
                    let bt = prop(Some(b), "type").and_then(Value::as_str);
                    if bt == Some("text") {
                        let raw = prop(Some(b), "text");
                        if self.prompt.is_none() { self.prompt = Some(truncate(&redact_val(raw), 6000, 1000)); changed = true; continue; }
                        let rs = if truthy(raw) { js::string(raw) } else { String::new() };
                        if SKIP_STR.is_match(js::trim(&rs)) { continue; }
                        let txt = redact_val(raw);
                        self.events.push(Event { t, kind: "user", summary: one_str(&txt, 140), text: Some(truncate(&txt, 4000, 500)), ..Default::default() });
                        changed = true;
                    } else if bt == Some("tool_result") {
                        let (text, images) = result_text(prop(Some(b), "content"));
                        let red = redact(&text);
                        let is_error = truthy(prop(Some(b), "is_error"));
                        let idx = self.by_tool_id.get(&key(prop(Some(b), "tool_use_id"))).copied();
                        if is_error { self.error_count += 1; }
                        if let Some(idx) = idx {
                            let ev = &mut self.events[idx];
                            ev.output = Some(truncate(&red, HEAD, TAIL));
                            ev.output_len = Some(len16(&red));
                            ev.is_error = Some(is_error);
                            ev.done = Some(true);
                            ev.duration_ms = Some(match (t, ev.t) { (Some(a), Some(b)) => Some((a - b).max(0.0)), _ => None });
                            ev.deny = Some(is_deny(&red));
                            if !images.is_empty() {
                                ev.images = Some(images.iter().map(|im| Image { data: String::new(), ..im.clone() }).collect());
                            }
                            let kept = images.into_iter().filter(|im| len16(&im.data) < 600_000).collect();
                            self.full.insert(idx, Full { output: clip_full(&red), images: kept });
                        } else {
                            self.events.push(Event {
                                t, kind: if is_error { "error" } else { "result" }, summary: one_str(&red, 140),
                                output: Some(truncate(&red, HEAD, TAIL)), is_error: Some(is_error), deny: Some(is_deny(&red)), ..Default::default()
                            });
                        }
                        changed = true;
                    }
                }
            }
            return changed;
        }
        // assistant
        if truthy(prop(r, "isApiErrorMessage")) {
            let txt = match content {
                Some(Value::Array(a)) => a.iter().map(|b| { let x = prop(Some(b), "text"); if truthy(x) { js::string(x) } else { String::new() } }).collect::<Vec<_>>().join(" "),
                c => js::string_or_empty(c),
            };
            self.events.push(Event { t, kind: "error", summary: redact(&one_str(&txt, 200)), is_error: Some(true), ..Default::default() });
            self.error_count += 1;
            return true;
        }
        let mut changed = false;
        if let Some(Value::Array(blocks)) = content {
            for b in blocks {
                match prop(Some(b), "type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(Value::String(s)) = prop(Some(b), "text") {
                            if js::trim(s).is_empty() { continue; }
                            let red = redact(s);
                            self.events.push(Event { t, kind: "text", summary: one_str(&red, 140), text: Some(truncate(&red, 12000, 2000)), ..Default::default() });
                            changed = true;
                        }
                    }
                    Some("thinking") | Some("redacted_thinking") => {
                        if let Some(last) = self.events.last_mut() { if last.kind == "thinking" { last.t2 = Some(t); continue; } }
                        self.events.push(Event { t, kind: "thinking", summary: "thinking…".into(), ..Default::default() });
                        changed = true;
                    }
                    Some("tool_use") => {
                        let raw_in = prop(Some(b), "input").filter(|v| !v.is_null());
                        let empty = Value::Object(Map::new());
                        let inp = raw_in.unwrap_or(&empty);
                        let name = prop(Some(b), "name");
                        self.by_tool_id.insert(key(prop(Some(b), "id")), self.events.len());
                        self.events.push(Event {
                            t, kind: "tool", tool: name.cloned(), id: prop(Some(b), "id").cloned(),
                            summary: summarize(&js::string(name), Some(inp)), input: Some(redact_value(inp, 4000)), done: Some(false),
                            raw_path_hints: path_hints(prop(Some(b), "input")), ..Default::default()
                        });
                        self.tool_count += 1;
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        changed
    }

    pub fn finished(&self) -> bool { truthy(prop(Some(&self.meta), "finished")) }

    pub fn status(&self, now: f64, stale: f64, stuck: f64) -> &'static str { compute_status(self, &self.meta, now, stale, stuck) }

    pub fn snapshot(&self, o: &Opts) -> Snapshot {
        let m = Some(&self.meta);
        let status = self.status(o.now, o.stale_sec, o.stuck_sec);
        let focus = eval_rules(self, o.rules, status);
        let final_text = self.events.iter().rev().find(|e| e.kind == "text").and_then(|e| e.text.clone()).unwrap_or_default();
        let at = prop(m, "agentType");
        let ty = if self.kind == "main" { Value::from("main session") } else if truthy(at) { at.unwrap().clone() } else { Value::from("agent") };
        let d = prop(m, "description");
        let description = if truthy(d) { d.unwrap().clone() } else { Value::from(if self.kind == "main" { "Main session" } else { "" }) };
        Snapshot {
            id: self.id.clone(), kind: self.kind, session_id: self.session_id.clone(), ty, description,
            prompt: self.prompt.clone().unwrap_or_default(), status, finished: self.finished(),
            started_at: self.started_at, last_event_at: self.last_event_at, tool_count: self.tool_count, error_count: self.error_count,
            event_count: self.events.len(), last_summary: self.events.last().map(|e| e.summary.clone()).unwrap_or_default(),
            final_text, spawn_depth: prop(m, "spawnDepth").cloned().unwrap_or(Value::Null),
            tool_use_id: prop(m, "toolUseId").cloned().unwrap_or(Value::Null), focus,
        }
    }

    /// `detail()`: snapshot + public events.
    pub fn detail(&self, o: &Opts) -> Value {
        let mut v = self.snapshot(o).to_value();
        v.as_object_mut().unwrap().insert("events".into(), Value::Array(self.events.iter().map(|e| e.to_value(false)).collect()));
        v
    }

    /// Events with the full (redacted, capped) output and images merged in, as `{...e, output, images}`.
    pub fn merged_event(&self, i: usize) -> Option<Event> {
        let e = self.events.get(i)?;
        let mut m = e.clone();
        match self.full.get(&i) {
            Some(f) => { m.output = Some(f.output.clone()); m.images = Some(f.images.clone()); }
            None => {}
        }
        Some(m)
    }

    /// `/api/agent/:id?event=i` body.
    pub fn event_value(&self, i: usize) -> Option<Value> {
        let e = self.events.get(i)?;
        let mut m = e.clone();
        let f = self.full.get(&i);
        if let Some(f) = f { m.output = Some(f.output.clone()); }
        m.images = Some(f.map(|f| f.images.clone()).unwrap_or_default());
        let mut o = Map::new();
        o.insert("event".into(), m.to_value(true));
        Some(Value::Object(o))
    }

    /// `/api/agent/:id?full=1` body.
    pub fn full_detail(&self, o: &Opts) -> Value {
        let mut v = self.snapshot(o).to_value();
        let evs = (0..self.events.len()).map(|i| {
            if self.full.contains_key(&i) { self.merged_event(i).unwrap().to_value(true) } else { self.events[i].to_value(true) }
        }).collect();
        v.as_object_mut().unwrap().insert("events".into(), Value::Array(evs));
        v
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub id: String,
    pub kind: &'static str,
    pub session_id: Option<String>,
    pub ty: Value,
    pub description: Value,
    pub prompt: String,
    pub status: &'static str,
    pub finished: bool,
    pub started_at: Option<f64>,
    pub last_event_at: Option<f64>,
    pub tool_count: usize,
    pub error_count: usize,
    pub event_count: usize,
    pub last_summary: String,
    pub final_text: String,
    pub spawn_depth: Value,
    pub tool_use_id: Value,
    pub focus: Vec<Hit>,
}

impl Snapshot {
    pub fn type_str(&self) -> String { js::string(Some(&self.ty)) }
    pub fn desc_str(&self) -> String { js::string(Some(&self.description)) }
    pub fn to_value(&self) -> Value {
        let mut m = Map::new();
        let f = |x: Option<f64>| x.map(Value::from).unwrap_or(Value::Null);
        m.insert("id".into(), Value::from(self.id.as_str()));
        m.insert("kind".into(), Value::from(self.kind));
        m.insert("sessionId".into(), self.session_id.as_deref().map(Value::from).unwrap_or(Value::Null));
        m.insert("type".into(), self.ty.clone());
        m.insert("description".into(), self.description.clone());
        m.insert("prompt".into(), Value::from(self.prompt.as_str()));
        m.insert("status".into(), Value::from(self.status));
        m.insert("finished".into(), Value::from(self.finished));
        m.insert("startedAt".into(), f(self.started_at));
        m.insert("lastEventAt".into(), f(self.last_event_at));
        m.insert("toolCount".into(), Value::from(self.tool_count as f64));
        m.insert("errorCount".into(), Value::from(self.error_count as f64));
        m.insert("eventCount".into(), Value::from(self.event_count as f64));
        m.insert("lastSummary".into(), Value::from(self.last_summary.as_str()));
        m.insert("finalText".into(), Value::from(self.final_text.as_str()));
        m.insert("spawnDepth".into(), self.spawn_depth.clone());
        m.insert("toolUseId".into(), self.tool_use_id.clone());
        m.insert("focus".into(), Value::Array(self.focus.iter().map(Hit::to_value).collect()));
        Value::Object(m)
    }
}

pub fn compute_status(b: &AgentBuilder, meta: &Value, now: f64, stale: f64, stuck: f64) -> &'static str {
    if truthy(prop(Some(meta), "finished")) { return "done"; }
    let age = match b.last_event_at { Some(l) => (now - l) / 1000.0, None => 0.0 };
    if age > stuck { return "stuck"; }
    if age > stale { return "stale"; }
    let n = b.events.len();
    for i in (n.saturating_sub(20)..n).rev() {
        let e = &b.events[i];
        if e.kind == "tool" && !e.is_done() { return "running"; }
        if e.kind == "text" || e.kind == "user" { break; }
    }
    "thinking"
}

// ---------------------------------------------------------------- focus rules

#[derive(Clone, Debug)]
pub struct Rule {
    pub name: Option<Value>,
    pub action: Value,
    pub tool: Option<Regex>,
    pub path: Option<Regex>,
    pub is_error: Option<bool>,
    pub deny: Option<bool>,
    pub stuck: bool,
}

#[derive(Clone, Debug)]
pub struct Hit { pub rule: Option<Value>, pub action: Value, pub idx: i64, pub t: Option<f64>, pub summary: String }

impl Hit {
    pub fn rule_str(&self) -> String { js::string(self.rule.as_ref()) }
    pub fn to_value(&self) -> Value {
        let mut m = Map::new();
        if let Some(r) = &self.rule { m.insert("rule".into(), r.clone()); }
        m.insert("action".into(), self.action.clone());
        m.insert("idx".into(), Value::from(self.idx as f64));
        m.insert("t".into(), self.t.map(Value::from).unwrap_or(Value::Null));
        m.insert("summary".into(), Value::from(self.summary.as_str()));
        Value::Object(m)
    }
}

fn js_regex(p: &str) -> Result<Regex, String> {
    RegexBuilder::new(p).case_insensitive(true).build().map_err(|e| e.to_string())
}

/// `compileRules`; Err when JS would throw (a non-object rule or an invalid pattern).
pub fn compile_rules(list: &Value) -> Result<Vec<Rule>, String> {
    let Value::Array(a) = list else { return Ok(vec![]) };
    a.iter().map(|r| {
        if r.is_null() { return Err("rule is null".to_string()); }
        let mo = prop(Some(r), "match");
        let empty = Value::Object(Map::new());
        let m = if truthy(mo) { mo.unwrap() } else { &empty };
        let act = prop(Some(r), "action");
        Ok(Rule {
            name: prop(Some(r), "name").cloned(),
            action: if truthy(act) { act.unwrap().clone() } else { Value::from("highlight") },
            tool: if truthy(prop(Some(m), "tool")) { Some(js_regex(&js::string(prop(Some(m), "tool")))?) } else { None },
            path: if truthy(prop(Some(m), "path")) { Some(js_regex(&js::string(prop(Some(m), "path")))?) } else { None },
            is_error: prop(Some(m), "isError").and_then(Value::as_bool),
            deny: prop(Some(m), "deny").and_then(Value::as_bool),
            stuck: prop(Some(m), "stuck") == Some(&Value::Bool(true)),
        })
    }).collect()
}

pub fn eval_rules(b: &AgentBuilder, rules: &[Rule], status: &str) -> Vec<Hit> {
    let mut hits = vec![];
    if rules.is_empty() { return hits; }
    for (idx, e) in b.events.iter().enumerate() {
        if e.kind != "tool" && e.kind != "result" && e.kind != "error" { continue; }
        for r in rules {
            if r.stuck { continue; }
            if let Some(t) = &r.tool {
                let ts = e.tool_str();
                if !(e.tool.as_ref().map(|v| truthy(Some(v))).unwrap_or(false) && t.is_match(&ts)) { continue; }
            }
            if let Some(p) = &r.path { if !p.is_match(&e.raw_path_hints) { continue; } }
            if let Some(ie) = r.is_error { if e.is_err() != ie { continue; } }
            if let Some(d) = r.deny { if e.is_deny() != d { continue; } }
            if r.tool.is_none() && r.path.is_none() && r.is_error.is_none() && r.deny.is_none() { continue; }
            hits.push(Hit { rule: r.name.clone(), action: r.action.clone(), idx: idx as i64, t: e.t, summary: e.summary.clone() });
        }
    }
    if status == "stuck" {
        for r in rules.iter().filter(|r| r.stuck) {
            hits.push(Hit { rule: r.name.clone(), action: r.action.clone(), idx: -1, t: b.last_event_at, summary: "no activity".into() });
        }
    }
    hits
}

/// One-shot parse (used by tests and the archive-style callers).
pub fn parse_agent_file(text: &str, meta: Value, id: Option<&str>, kind: &'static str, session_id: Option<String>, o: &Opts) -> Value {
    let mid = prop(Some(&meta), "agentId");
    let id0 = match id { Some(s) if !s.is_empty() => Some(s.to_string()), _ => if truthy(mid) { Some(js::string(mid)) } else { None } };
    let mut b = AgentBuilder::new(id0.as_deref().unwrap_or(""), kind, session_id);
    b.meta = if truthy(Some(&meta)) { meta } else { Value::Object(Map::new()) };
    for r in split_lines(text).records { b.push(&r); }
    if id0.is_none() { b.id = if truthy(b.agent_id.as_ref()) { js::string(b.agent_id.as_ref()) } else { "unknown".into() }; }
    b.detail(o)
}

pub fn project_dir_name(p: &str) -> String {
    p.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_string() } else { "-".repeat(c.len_utf16()) }).collect()
}
