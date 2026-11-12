//! Port of src/collection.mjs: the server-rendered Collection page, its body fragment and the plain-text snapshot.
//! The static shell (CSS, header, client script, Watchkeeper text) lives in rust/assets, extracted from the Node module
//! by parity/extract-assets.mjs so the two can never drift silently.
use crate::js::{self, esc_html as esc, iso as iso_ms, len16};
use crate::parse::{AgentBuilder, Event, Full, Image, Snapshot};
use crate::store::State;
use regex::Regex;
use serde_json::Value;
use std::fmt::Write as _;
use std::sync::LazyLock;

pub const SCHEMA_VERSION: &str = "agentwatch-collection/1";
pub const WATCHKEEPER_PROTOCOL: &str = include_str!("../assets/watchkeeper.txt");
const PAGE: &str = include_str!("../assets/collection_page.html");

fn iso(ms: Option<f64>) -> String { match ms { Some(v) if v != 0.0 && !v.is_nan() => iso_ms(v), _ => String::new() } }
fn hm(ms: Option<f64>) -> String { let s = iso(ms); if s.is_empty() { s } else { format!("{}Z", js::slice16(&s, 11, 19)) } }

fn dur(ms: f64) -> String {
    if ms < 1000.0 { return format!("{} ms", js::num(ms)); }
    let s = ms / 1000.0;
    if s < 60.0 { format!("{} s", js::to_fixed1(s)) } else { format!("{} min {} s", js::num((s / 60.0).floor()), js::num(js::round(s % 60.0))) }
}

pub fn dom_id(id: &str) -> String {
    let mut o = String::from("agent-");
    for c in id.chars() { if c.is_ascii_alphanumeric() || c == '_' || c == '-' { o.push(c) } else { for _ in 0..c.len_utf16() { o.push('-') } } }
    o
}

fn status_word(s: &str) -> &str {
    match s {
        "running" => "RUNNING (tool in flight)", "thinking" => "THINKING (waiting on model)", "stale" => "STALE (no activity > stale threshold)",
        "stuck" => "STUCK (no activity > stuck threshold)", "done" => "DONE (finished)", s => s,
    }
}

fn re(p: &str) -> Regex { Regex::new(p).unwrap() }
static HANDBACK: LazyLock<Regex> = LazyLock::new(|| re(r"\[Subagent hand-back\]|<agent-message from="));
static TASKNOTE: LazyLock<Regex> = LazyLock::new(|| re(r"<task-notification>"));
static NOISE_TOOLS: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:ToolSearch|TodoWrite|TaskOutput|ListAgents)$"));
static SIG_TOOLS: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:Edit|Write|MultiEdit|NotebookEdit|Agent|Task|Skill)$"));
static SIG_WORDS: LazyLock<Regex> = LazyLock::new(|| re(r"(?-u:\b)(?i-u:test|scorecard|bench|smoke|evaluate|publish|commit)(?-u:\b)"));
static ELISION: LazyLock<Regex> = LazyLock::new(|| re(r"… \[([0-9]+) (?:chars trimmed|more chars not kept)\]"));

fn event_type(e: &Event, agent_kind: &str) -> &'static str {
    match e.kind {
        "thinking" => "Thought",
        "text" => "Message",
        "user" => {
            let t = e.text.as_deref().filter(|s| !s.is_empty()).unwrap_or(&e.summary);
            if HANDBACK.is_match(t) { "Subagent handback" } else if TASKNOTE.is_match(t) { "Task notification" } else if agent_kind == "main" { "User message" } else { "Message in" }
        }
        "error" => "Error",
        "result" => "Tool result",
        _ => "Tool call",
    }
}

fn signal_of(ty: &str, e: &Event) -> &'static str {
    if ty == "Thought" { return "noise"; }
    let tool = e.tool_str();
    if ty == "Tool call" && NOISE_TOOLS.is_match(&tool) { return "noise"; }
    if ["Subagent handback", "Task notification", "Error", "User message"].contains(&ty) || e.is_err() || e.is_deny() { return "significant"; }
    if ty == "Tool call" && SIG_TOOLS.is_match(&tool) { return "significant"; }
    if ty == "Tool call" && SIG_WORDS.is_match(&e.summary) { return "significant"; }
    "activity"
}

fn error_status(events: &[Event], i: usize, status: &str) -> (String, String) {
    let e = &events[i];
    if e.is_deny() { return ("blocked-by-guard".into(), "a hook or guard denied this call (policy, not a crash)".into()); }
    if let Some(j) = (i + 1..events.len()).find(|&j| { let x = &events[j]; x.kind == "tool" && x.is_done() && !x.is_err() && x.tool == e.tool }) {
        return ("recovered".into(), format!("a later {} call succeeded (event #{})", js::string(e.tool.as_ref()), j + 1));
    }
    if status == "done" {
        let t = if js::truthy(e.tool.as_ref()) { e.tool_str() } else { "call".into() };
        return ("unresolved".into(), format!("no later successful {t} before the agent finished"));
    }
    ("unknown".into(), "agent still working; no later success yet".into())
}

fn clip_note(shown: usize, total: Option<usize>, what: &str) -> String {
    match total {
        Some(t) if t > shown => format!("{what} CLIPPED: showing {shown} of {t} characters. Reason: Agent Watch keeps at most 20 KB per tool result and trims long inputs/messages (head + tail kept; the elided middle is marked inline)."),
        _ => String::new(),
    }
}

fn elision_note(s: &str) -> String {
    match ELISION.captures(s) { Some(c) => format!("contains an inline elision marker: {} characters omitted", &c[1]), None => String::new() }
}

fn first(s: &str, n: usize) -> String {
    let mut c = String::with_capacity(s.len());
    let mut in_ws = false;
    for ch in s.chars() {
        if js::is_ws(ch) { if !in_ws { c.push(' '); in_ws = true; } } else { c.push(ch); in_ws = false; }
    }
    let t = js::trim(&c);
    if len16(t) > n { format!("{}…", js::head16(t, n - 1)) } else { t.to_string() }
}

fn pre(s: &str, cls: &str) -> String { let mut o = String::with_capacity(s.len() + 32); pre_into(&mut o, s, cls); o }
fn pre_into(o: &mut String, s: &str, cls: &str) { o.push_str("<pre class=\""); o.push_str(cls); o.push_str("\">"); js::esc_html_into(o, s); o.push_str("</pre>"); }
fn vstr(v: &Value) -> String { js::string_or_empty(Some(v)) }

// ---------------------------------------------------------------- model

/// A stored event seen through `{...e, output: full.output, images: full.images}` without copying anything.
#[derive(Clone, Copy)]
pub struct Ev<'a> { e: &'a Event, full: Option<&'a Full> }

impl<'a> std::ops::Deref for Ev<'a> {
    type Target = Event;
    fn deref(&self) -> &Event { self.e }
}

impl<'a> Ev<'a> {
    pub fn out(&self) -> Option<&'a str> { match self.full { Some(f) => Some(f.output.as_str()), None => self.e.output.as_deref() } }
    pub fn imgs(&self) -> Option<&'a [Image]> { match self.full { Some(f) => Some(f.images.as_slice()), None => self.e.images.as_deref() } }
}

pub struct MEvent<'a> { pub n: usize, pub id: String, pub t: Option<f64>, pub ty: &'static str, pub signal: &'static str, pub e: Ev<'a>, pub err: Option<(String, String)> }

pub struct MAgent<'a> { pub s: Snapshot, pub prompt: String, pub events: Vec<MEvent<'a>>, pub handback: String, pub latest: Option<usize>, pub dom_id: String }

impl MAgent<'_> {
    fn desc_or_id(&self) -> String { if js::truthy(Some(&self.s.description)) { self.s.desc_str() } else { self.s.id.clone() } }
}

pub struct Model<'a> {
    pub now: f64,
    pub version: String,
    pub session_ids: Vec<String>,
    pub agents: Vec<MAgent<'a>>,
    pub counts: [usize; 5],
    pub err_by: Vec<(String, usize)>,
    pub total_errors: usize,
    pub in_flight: Vec<(usize, usize)>,
    pub last_tool: Option<(usize, usize)>,
    pub handbacks: Vec<(usize, usize)>,
    pub finished: Vec<usize>,
    pub main: Option<usize>,
    pub total_events: usize,
    pub latest_event_at: Option<f64>,
}

impl<'a> Model<'a> {
    fn err_count(&self, k: &str) -> usize { self.err_by.iter().find(|(s, _)| s == k).map(|x| x.1).unwrap_or(0) }
    fn ev(&self, (a, x): (usize, usize)) -> (&MAgent<'a>, &MEvent<'a>) { (&self.agents[a], &self.agents[a].events[x]) }
}

/// `details(id)` returns the agent's builder (for its prompt and merged events), like `store.agent(id, null, {full})`.
pub fn build_model<'a>(state: State, details: impl Fn(&str) -> Option<&'a AgentBuilder>, now: f64) -> Model<'a> {
    let session_ids = state.session_ids;
    let mut agents: Vec<MAgent<'a>> = state.agents.into_iter().map(|a| {
        let b = details(&a.id);
        let (prompt, evs): (String, &'a [Event]) = match b {
            Some(b) => (b.prompt.clone().unwrap_or_default(), b.events.as_slice()),
            None => (a.prompt.clone(), &[]),
        };
        let did = dom_id(&a.id);
        let events: Vec<MEvent<'a>> = evs.iter().enumerate().map(|(i, e)| {
            let ty = event_type(e, a.kind);
            let mut err = None;
            if e.kind == "tool" && e.is_err() { err = Some(error_status(evs, i, a.status)); }
            if e.kind == "error" { err = Some((if a.status == "done" { "unresolved" } else { "unknown" }.into(), "agent-level error (API or system)".into())); }
            let full = b.and_then(|b| b.full.get(&i));
            MEvent { n: i + 1, id: format!("{did}-e{}", i + 1), t: e.t, ty, signal: signal_of(ty, e), e: Ev { e, full }, err }
        }).collect();
        let handback = if a.kind == "agent" && a.status == "done" { a.final_text.clone() } else { String::new() };
        let latest = events.iter().rposition(|x| x.signal != "noise");
        MAgent { s: a, prompt, events, handback, latest, dom_id: did }
    }).collect();
    let rank = |a: &MAgent| if a.s.kind == "main" { 0 } else if ["running", "thinking", "stale", "stuck"].contains(&a.s.status) { 1 } else { 2 };
    agents.sort_by(|x, y| rank(x).cmp(&rank(y)).then_with(|| y.s.last_event_at.unwrap_or(0.0).partial_cmp(&x.s.last_event_at.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal)));
    let mut counts = [0usize; 5];
    for a in agents.iter().filter(|a| a.s.kind != "main") {
        if let Some(i) = ["running", "thinking", "stale", "stuck", "done"].iter().position(|s| *s == a.s.status) { counts[i] += 1; }
    }
    let mut err_by: Vec<(String, usize)> = vec![];
    let mut total_errors = 0;
    let mut in_flight = vec![];
    let mut tools = vec![];
    let mut hb = vec![];
    for (ai, a) in agents.iter().enumerate() {
        for (xi, x) in a.events.iter().enumerate() {
            if let Some((st, _)) = &x.err {
                total_errors += 1;
                match err_by.iter_mut().find(|(s, _)| s == st) { Some(e) => e.1 += 1, None => err_by.push((st.clone(), 1)) }
            }
            if x.e.kind == "tool" { tools.push((ai, xi)); if !x.e.is_done() { in_flight.push((ai, xi)); } }
            if x.ty == "Subagent handback" || x.ty == "Task notification" { hb.push((ai, xi)); }
        }
    }
    let t_of = |p: &(usize, usize)| agents[p.0].events[p.1].t.unwrap_or(0.0);
    tools.sort_by(|p, q| t_of(q).partial_cmp(&t_of(p)).unwrap_or(std::cmp::Ordering::Equal));
    hb.sort_by(|p, q| t_of(q).partial_cmp(&t_of(p)).unwrap_or(std::cmp::Ordering::Equal));
    hb.truncate(6);
    let mut finished: Vec<usize> = (0..agents.len()).filter(|&i| { let a = &agents[i]; a.s.kind == "agent" && a.s.status == "done" && !a.handback.is_empty() }).collect();
    finished.sort_by(|&p, &q| agents[q].s.last_event_at.unwrap_or(0.0).partial_cmp(&agents[p].s.last_event_at.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
    finished.truncate(6);
    let main = agents.iter().position(|a| a.s.kind == "main");
    let sig = agents.iter().map(|a| format!("{}:{}:{}:{}", a.s.id, a.s.status, a.events.len(), js::num_opt(a.s.last_event_at))).collect::<Vec<_>>().join("|");
    let version = sha1_smol::Sha1::from(sig.as_bytes()).digest().to_string()[..10].to_string();
    let total_events = agents.iter().map(|a| a.events.len()).sum();
    let lmax = agents.iter().map(|a| a.s.last_event_at.filter(|v| *v != 0.0 && !v.is_nan()).unwrap_or(0.0)).fold(0.0f64, f64::max);
    Model {
        now, version, session_ids, last_tool: tools.first().copied(), agents, counts, err_by, total_errors, in_flight,
        handbacks: hb, finished, main, total_events, latest_event_at: if lmax != 0.0 { Some(lmax) } else { None },
    }
}

// ---------------------------------------------------------------- HTML

fn render_snapshot(m: &Model<'_>, o: &mut String) {
    let st = match m.main {
        Some(i) => { let a = &m.agents[i].s; format!("{} · last activity {} · {} tool calls", a.status.to_ascii_uppercase(), iso(a.last_event_at), a.tool_count) }
        None => "not tracked".into(),
    };
    let now = iso(Some(m.now));
    let sessions = if m.session_ids.is_empty() { "none".to_string() } else { m.session_ids.iter().map(|s| format!("<code>{}</code>", esc(s))).collect::<Vec<_>>().join(", ") };
    let inflight = if m.in_flight.is_empty() { "none".to_string() } else {
        m.in_flight.iter().map(|&p| { let (a, x) = m.ev(p); format!("<a href=\"#{}\">{} #{}: {}</a> (started {})", x.id, esc(&a.s.type_str()), x.n, esc(&x.e.summary), iso(x.t)) }).collect::<Vec<_>>().join("<br>")
    };
    let last = match m.last_tool { Some(p) => { let (a, x) = m.ev(p); format!("<a href=\"#{}\">{} #{}: {}</a> at {}", x.id, esc(&a.s.type_str()), x.n, esc(&x.e.summary), iso(x.t)) } None => "none".into() };
    let le = iso(m.latest_event_at);
    let hbs: String = m.handbacks.iter().map(|&p| {
        let (_, x) = m.ev(p);
        let txt = x.e.text.as_deref().filter(|s| !s.is_empty()).unwrap_or(&x.e.summary);
        format!("<li><a href=\"#{}\">{}</a> at {}: {}</li>", x.id, esc(x.ty), iso(x.t), esc(&first(txt, 260)))
    }).collect();
    let fin: String = m.finished.iter().map(|&i| {
        let a = &m.agents[i];
        format!("<li><a href=\"#{}\">{}: {}</a> finished {}: {}</li>", a.dom_id, esc(&a.s.type_str()), esc(&a.s.desc_str()), iso(a.s.last_event_at), esc(&first(&a.handback, 260)))
    }).collect();
    let c = m.counts;
    let _ = write!(o, r#"<section id="current-snapshot" aria-labelledby="h-snapshot" class="card">
<h2 id="h-snapshot">Current snapshot (derived deterministically from transcripts; not AI-generated)</h2>
<dl class="kv">
<dt>Snapshot rendered at</dt><dd><time datetime="{now}">{now}</time></dd>
<dt>Snapshot version</dt><dd data-snapshot-version="{v}">{v} (changes whenever any agent's status, event count or last activity changes)</dd>
<dt>Schema version</dt><dd data-schema-version="{sc}">{sc}</dd>
<dt>Sessions on this page</dt><dd>{sessions}</dd>
<dt>Main session status</dt><dd>{st}</dd>
<dt>Subagents by status</dt><dd>running {c0} · thinking {c1} · stale {c2} · stuck {c3} · done {c4}</dd>
<dt>Currently executing tool calls</dt><dd>{inflight}</dd>
<dt>Most recent tool call</dt><dd>{last}</dd>
<dt>Errors (all agents)</dt><dd>{te} total · recovered {er} · unresolved {eu} · unknown {ek} · blocked by guard {eg}</dd>
<dt>Changed since a prior read</dt><dd>Every event has a sequence number (#n per agent) and an ISO timestamp. Latest event on this page: <time datetime="{le}">{le}</time>. Total events: {tev}. Compare the snapshot version and per-agent event counts with your previous reading.</dd>
</dl>
<h3>Most recent handbacks and notifications (main session)</h3>
<ol>{hbs}</ol>
<h3>Most recently finished subagents (first line of their final report)</h3>
<ol>{fin}</ol>
</section>"#,
        v = m.version, sc = SCHEMA_VERSION, st = esc(&st), c0 = c[0], c1 = c[1], c2 = c[2], c3 = c[3], c4 = c[4], te = m.total_errors,
        er = m.err_count("recovered"), eu = m.err_count("unresolved"), ek = m.err_count("unknown"), eg = m.err_count("blocked-by-guard"), tev = m.total_events,
        hbs = if hbs.is_empty() { "<li>none</li>".into() } else { hbs }, fin = if fin.is_empty() { "<li>none</li>".into() } else { fin });
}

fn render_index(m: &Model<'_>, o: &mut String) {
    let rows = m.agents.iter().enumerate().map(|(i, a)| {
        let latest = match a.latest { Some(l) => { let x = &a.events[l]; format!("#{} {}: {}", x.n, esc(x.ty), esc(&first(&x.e.summary, 120))) } None => String::new() };
        format!("<tr><td>{}</td><td><a href=\"#{}\">{}: {}</a></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            i + 1, a.dom_id, esc(&a.s.type_str()), esc(&a.desc_or_id()), if a.s.kind == "main" { "main session (orchestrator)" } else { "subagent" },
            esc(&a.s.status.to_ascii_uppercase()), iso(a.s.started_at), dur(a.s.last_event_at.unwrap_or(0.0) - a.s.started_at.unwrap_or(0.0)),
            a.s.tool_count, a.s.error_count, a.events.len(), iso(a.s.last_event_at), latest)
    }).collect::<Vec<_>>().join("\n");
    let _ = write!(o, r#"<section id="summary-index" aria-labelledby="h-index" class="card">
<h2 id="h-index">Summary index ({} agents, main session first)</h2>
<table><thead><tr><th>#</th><th>agent</th><th>kind</th><th>status</th><th>started</th><th>elapsed</th><th>tool calls</th><th>errors</th><th>events</th><th>last activity</th><th>latest meaningful activity</th></tr></thead><tbody>
{rows}
</tbody></table></section>"#, m.agents.len());
}

fn render_event(a: &MAgent<'_>, x: &MEvent<'_>, o: &mut String) {
    let e = &x.e;
    let tool_span = if js::truthy(e.tool.as_ref()) { format!(" <span class=\"tool\">{}</span>", esc(&e.tool_str())) } else { String::new() };
    let head = format!("<span class=\"evn\">#{}</span> <span class=\"evtype\">{}</span>{tool_span} <span class=\"signal\">[{}]</span> <time datetime=\"{}\">{}</time>", x.n, esc(x.ty), x.signal, iso(x.t), hm(x.t));
    let attrs = format!("id=\"{}\" class=\"ev {}{}\" data-seq=\"{}\" data-type=\"{}\" data-signal=\"{}\" data-t=\"{}\"", x.id, x.signal, if e.is_err() { " error" } else { "" }, x.n, esc(x.ty), x.signal,
        x.t.map(js::num).unwrap_or_default());
    if x.ty == "Thought" {
        let _ = write!(o, "<div {attrs}><p class=\"evhead\">{head} · thinking (content not recorded by Claude Code)</p></div>");
        return;
    }
    if e.kind == "tool" {
        let corr = if js::truthy(e.id.as_ref()) { js::string(e.id.as_ref()) } else { format!("{}-c", x.id) };
        let out = e.out().unwrap_or("");
        let out_shown = len16(out);
        let status = if !e.is_done() { "PENDING (no result yet)".to_string() } else if e.is_err() {
            let (s, w) = x.err.as_ref().unwrap(); format!("ERROR · {} ({w})", s.to_ascii_uppercase())
        } else { "OK".into() };
        let all_imgs = e.imgs().unwrap_or(&[]);
        let imgs: Vec<_> = all_imgs.iter().filter(|im| !im.data.is_empty()).collect();
        static EMPTY: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| Value::Object(Default::default()));
        let input: &Value = e.input.as_ref().unwrap_or(&EMPTY);
        let in_compact = match &e.input { Some(v) => js::json(v), None => String::new() };
        let in_note = elision_note(&in_compact);
        let cn = clip_note(out_shown, e.output_len, "Output");
        let on = elision_note(out);
        let c = esc(&corr);
        let _ = write!(o, "<div {attrs} data-correlation-id=\"{c}\">
<p class=\"evhead\">{head} · {}</p>
<div class=\"pair\">
<div class=\"call\" id=\"{id}-call\" data-correlation-id=\"{c}\"><p class=\"lbl\">Tool call · correlation id <code>{c}</code></p>",
            esc(&e.summary), id = x.id);
        pre_into(o, &js::json_pretty(input), "");
        if !in_note.is_empty() { let _ = write!(o, "<p class=\"clip\">Input {} (strings over 4000 characters are trimmed, head and tail kept).</p>", esc(&in_note)); }
        let _ = write!(o, "</div>
<div class=\"result\" id=\"{id}-result\" data-correlation-id=\"{c}\"><p class=\"lbl\">Tool result · {}", esc(&status), id = x.id);
        if let Some(Some(d)) = e.duration_ms { let _ = write!(o, " · took {}", dur(d)); }
        if let Some(n) = e.output_len { let _ = write!(o, " · {n} characters"); }
        o.push_str("</p>
");
        if e.is_done() { pre_into(o, out, if e.is_err() { "bad" } else { "" }); } else { o.push_str("<p>no result yet</p>"); }
        o.push('\n');
        if !cn.is_empty() { let _ = write!(o, "<p class=\"clip\">{}</p>", esc(&cn)); }
        if !on.is_empty() { let _ = write!(o, "<p class=\"clip\">Output {}.</p>", esc(&on)); }
        o.push('\n');
        for (k, im) in imgs.iter().enumerate() {
            let _ = write!(o, "<figure><img loading=\"lazy\" alt=\"Image {} of {} returned by {} in event #{} ({}, about {} KB)\" src=\"/api/image/{}/{}/{}\"><figcaption>Image {} of {} from event #{} ({}: {})</figcaption></figure>",
                k + 1, imgs.len(), esc(&e.tool_str()), x.n, esc(&im.media_type), js::num(js::round(len16(&im.data) as f64 * 0.75 / 1024.0)),
                js::encode_uri_component(&a.s.id), x.n - 1, k, k + 1, imgs.len(), x.n, esc(&e.tool_str()), esc(&first(&e.summary, 80)));
        }
        o.push('\n');
        if all_imgs.len() > imgs.len() { let _ = write!(o, "<p class=\"clip\">{} image(s) not kept (over 600 KB).</p>", all_imgs.len() - imgs.len()); }
        o.push_str("
</div></div></div>");
        return;
    }
    let text = [e.text.as_deref(), e.out(), Some(e.summary.as_str())].into_iter().flatten().find(|s| !s.is_empty()).unwrap_or("");
    let err_line = match &x.err { Some((s, w)) => format!("<p class=\"lbl\">Error status: {} ({})</p>", esc(&s.to_ascii_uppercase()), esc(w)), None => String::new() };
    let en = elision_note(text);
    let _ = write!(o, "<div {attrs}><p class=\"evhead\">{head}</p>{err_line}");
    pre_into(o, text, if e.is_err() { "bad" } else { "msg" });
    if !en.is_empty() { let _ = write!(o, "<p class=\"clip\">{} (long messages keep head and tail).</p>", esc(&en)); }
    o.push_str("</div>");
}

fn render_agent(a: &MAgent<'_>, o: &mut String) {
    let s = &a.s;
    let elapsed = dur(s.last_event_at.unwrap_or(0.0) - s.started_at.unwrap_or(0.0));
    let meta = format!("{}: {} · status {} · elapsed {elapsed} · {} tool calls · {} errors · last activity {}",
        if s.kind == "main" { "Main session (orchestrator)".to_string() } else { format!("Subagent {}", esc(&s.type_str())) },
        esc(&a.desc_or_id()), esc(&s.status.to_ascii_uppercase()), s.tool_count, s.error_count, iso(s.last_event_at));
    let sid = s.session_id.clone().unwrap_or_default();
    let spawned = if js::truthy(Some(&s.tool_use_id)) { format!(" · spawned by tool call <code>{}</code>", esc(&vstr(&s.tool_use_id))) } else { String::new() };
    let latest = match a.latest { Some(l) => { let x = &a.events[l]; format!("<a href=\"#{}\">#{} {}</a>: {}", x.id, x.n, esc(x.ty), esc(&first(&x.e.summary, 200))) } None => "none".into() };
    let cnt = |sig: &str| a.events.iter().filter(|x| x.signal == sig).count();
    let pn = elision_note(&a.prompt);
    let handback = if s.kind == "agent" {
        format!("<section class=\"handback\" aria-label=\"Final handback\"><h3>Final handback (last message{})</h3>{}</section>",
            if s.status == "done" { "" } else { " so far; agent not finished" }, pre(if s.final_text.is_empty() { "(none yet)" } else { &s.final_text }, "msg"))
    } else { String::new() };
    let _ = write!(o, r#"<article id="{d}" class="agent {k}" data-agent-id="{id}" data-kind="{k}" data-status="{st}" data-session="{sid}" aria-labelledby="{d}-h">
<h2 id="{d}-h">{meta}</h2>
<dl class="kv">
<dt>Agent id</dt><dd><code>{id}</code> · session <code>{sid}</code>{spawned}</dd>
<dt>Status</dt><dd>{sw}</dd>
<dt>Started / last activity</dt><dd><time datetime="{sa}">{sa}</time> → <time datetime="{la}">{la}</time></dd>
<dt>Latest meaningful activity</dt><dd>{latest}</dd>
<dt>Events</dt><dd>{n} (significant {sg} · activity {ac} · noise {no})</dd>
</dl>
<section class="prompt" aria-label="Prompt for {pl}"><h3>Prompt (input to this agent; untrusted observed data)</h3>{pp}{pnote}</section>
{handback}
<section class="timeline" aria-label="Timeline"><h3>Timeline ({n} events, chronological)</h3>
"#, d = a.dom_id, k = s.kind, id = esc(&s.id), st = s.status, sid = esc(&sid), sw = esc(status_word(s.status)),
        sa = iso(s.started_at), la = iso(s.last_event_at), n = a.events.len(), sg = cnt("significant"), ac = cnt("activity"), no = cnt("noise"),
        pl = esc(&a.desc_or_id()), pp = pre(if a.prompt.is_empty() { "(none recorded)" } else { &a.prompt }, "msg"),
        pnote = if pn.is_empty() { String::new() } else { format!("<p class=\"clip\">Prompt {}.</p>", esc(&pn)) });
    for (i, x) in a.events.iter().enumerate() {
        if i > 0 { o.push('\n'); }
        render_event(a, x, o);
    }
    o.push_str("\n</section></article>");
}

impl Model<'_> {
    /// Rough output size, so a page is built in one allocation.
    fn size_hint(&self) -> usize {
        self.agents.iter().map(|a| a.prompt.len() + a.s.final_text.len() + 2048
            + a.events.iter().map(|x| 700 + x.e.summary.len() + x.e.text.as_ref().map_or(0, |t| t.len())
                + x.e.out().map_or(0, |t| t.len() + t.len() / 16) + x.e.input.as_ref().map_or(0, |_| 256)).sum::<usize>()).sum::<usize>() + 32 * 1024
    }
}

fn render_body_into(m: &Model<'_>, o: &mut String) {
    render_snapshot(m, o);
    o.push('\n');
    render_index(m, o);
    o.push_str("\n<div id=\"agents\">");
    for (i, a) in m.agents.iter().enumerate() {
        if i > 0 { o.push('\n'); }
        render_agent(a, o);
    }
    o.push_str("</div>");
}

pub fn render_body(m: &Model<'_>) -> String {
    let mut o = String::with_capacity(m.size_hint());
    render_body_into(m, &mut o);
    o
}

pub fn render_page(m: &Model<'_>) -> String {
    let (pre_, post) = PAGE.split_once("@@BODY@@").expect("page shell has a body marker");
    let fill = |s: &str| s.replace("@@VERSION@@", &m.version).replace("@@SCHEMA@@", SCHEMA_VERSION).replace("@@NOW@@", &iso(Some(m.now)));
    let mut o = String::with_capacity(PAGE.len() + m.size_hint());
    o.push_str(&fill(pre_));
    render_body_into(m, &mut o);
    o.push_str(&fill(post));
    o
}

// ---------------------------------------------------------------- text snapshot

/// Lines joined with "\n", written straight into one buffer.
struct Lines(String, bool);

impl Lines {
    fn push(&mut self, s: impl AsRef<str>) { if self.1 { self.0.push('\n'); } self.1 = true; self.0.push_str(s.as_ref()); }
    fn extend<S: AsRef<str>>(&mut self, it: impl IntoIterator<Item = S>) { for s in it { self.push(s); } }
}

pub fn snapshot_text(m: &Model<'_>) -> String {
    let mut l = Lines(String::with_capacity(m.size_hint()), false);
    let bar = "=".repeat(80);
    l.extend([WATCHKEEPER_PROTOCOL.to_string(), String::new(), bar.clone(), format!("AGENT WATCH COLLECTION SNAPSHOT · version {} · schema {SCHEMA_VERSION} · rendered {}", m.version, iso(Some(m.now))), bar]);
    let main = match m.main { Some(i) => { let a = &m.agents[i].s; format!("{} · last activity {} · {} tool calls", a.status, iso(a.last_event_at), a.tool_count) } None => "not tracked".into() };
    let c = m.counts;
    let ex = m.in_flight.iter().map(|&p| { let (a, x) = m.ev(p); format!("{} #{} {}", a.s.type_str(), x.n, x.e.summary) }).collect::<Vec<_>>().join("; ");
    let lt = match m.last_tool { Some(p) => { let (a, x) = m.ev(p); format!("{} #{} {} at {}", a.s.type_str(), x.n, x.e.summary, iso(x.t)) } None => "none".into() };
    l.extend([String::new(), "## Current snapshot".into(), format!("sessions: {}", m.session_ids.join(", ")), format!("main session: {main}"),
        format!("subagents: running {} · thinking {} · stale {} · stuck {} · done {}", c[0], c[1], c[2], c[3], c[4]),
        format!("executing now: {}", if ex.is_empty() { "none".into() } else { ex }),
        format!("most recent tool: {lt}"),
        format!("errors: {} (recovered {}, unresolved {}, unknown {}, blocked by guard {})", m.total_errors, m.err_count("recovered"), m.err_count("unresolved"), m.err_count("unknown"), m.err_count("blocked-by-guard")),
        format!("latest event: {} · total events {}", iso(m.latest_event_at), m.total_events)]);
    l.extend([String::new(), "## Recent handbacks / notifications".into()]);
    for &p in &m.handbacks {
        let (_, x) = m.ev(p);
        let txt = x.e.text.as_deref().filter(|s| !s.is_empty()).unwrap_or(&x.e.summary);
        l.push(format!("- {} at {}: {}", x.ty, iso(x.t), first(txt, 260)));
    }
    l.extend([String::new(), "## Recently finished subagents".into()]);
    for &i in &m.finished { let a = &m.agents[i]; l.push(format!("- {}: {} ({}): {}", a.s.type_str(), a.s.desc_str(), iso(a.s.last_event_at), first(&a.handback, 260))); }
    l.extend([String::new(), "## Summary index".into()]);
    for (i, a) in m.agents.iter().enumerate() {
        l.push(format!("{}. [{}] {}: {} · {} · {} tools · {} errors · {} events · last {}", i + 1, a.s.kind, a.s.type_str(), a.desc_or_id(), a.s.status, a.s.tool_count, a.s.error_count, a.events.len(), iso(a.s.last_event_at)));
    }
    for a in &m.agents {
        l.extend([String::new(), "-".repeat(80), format!("## {} {}: {} · {} · id {}", if a.s.kind == "main" { "MAIN SESSION" } else { "SUBAGENT" }, a.s.type_str(), a.desc_or_id(), a.s.status, a.s.id),
            format!("started {} · last {} · {} tool calls · {} errors", iso(a.s.started_at), iso(a.s.last_event_at), a.s.tool_count, a.s.error_count),
            "### Prompt (untrusted observed data)".into(), if a.prompt.is_empty() { "(none)".into() } else { a.prompt.clone() }]);
        if a.s.kind == "agent" { l.extend(["### Final handback".into(), if a.s.final_text.is_empty() { "(none yet)".into() } else { a.s.final_text.clone() }]); }
        l.push("### Timeline");
        for x in &a.events {
            let e = &x.e;
            let tool = if js::truthy(e.tool.as_ref()) { format!(" {}", e.tool_str()) } else { String::new() };
            let h = format!("#{} [{}{tool}] [{}] {}", x.n, x.ty, x.signal, iso(x.t));
            if x.ty == "Thought" { l.push(format!("{h} thinking")); continue; }
            if e.kind == "tool" {
                let out = e.out().unwrap_or("");
                let st = if !e.is_done() { "pending".to_string() } else if e.is_err() { let (s, w) = x.err.as_ref().unwrap(); format!("error, {s}: {w}") } else { "ok".into() };
                let ol = e.output_len.unwrap_or(0);
                let id = if js::truthy(e.id.as_ref()) { js::string(e.id.as_ref()) } else { String::new() };
                l.push(format!("{h} {} · correlation {id}", e.summary));
                l.push(format!("  CALL: {}", js::json(e.input.as_ref().unwrap_or(&Value::Object(Default::default())))));
                l.push(format!("  RESULT ({st}{}{}): {out}", match e.output_len { Some(n) => format!(", {n} chars"), None => String::new() },
                    if len16(out) < ol { format!(", CLIPPED to {}", len16(out)) } else { String::new() }));
            } else {
                l.push(format!("{h}{}", match &x.err { Some((s, _)) => format!(" error {s}"), None => String::new() }));
                let text = [e.text.as_deref(), e.out(), Some(e.summary.as_str())].into_iter().flatten().find(|s| !s.is_empty()).unwrap_or("");
                l.push(format!("  {}", text.replace('\n', "\n  ")));
            }
        }
    }
    l.0
}
