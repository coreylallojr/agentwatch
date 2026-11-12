//! Port of the Store class in src/server.mjs: discovers sessions under the projects root, tails transcript JSONL
//! incrementally, and answers state/agent queries. Snapshots that Node would emit through `onAgent` are queued in
//! `emitted` for the server to broadcast.
use crate::clock::now_ms;
use crate::js::{self, prop};
use crate::parse::{AgentBuilder, Opts, Rule, Snapshot, STALE_SEC, STUCK_SEC};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub const DAY: f64 = 24.0 * 3600.0 * 1000.0;

pub fn mtime_ms(md: &fs::Metadata) -> f64 {
    match md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()) {
        Some(d) => d.as_secs() as f64 * 1000.0 + d.subsec_nanos() as f64 / 1e6,
        None => 0.0,
    }
}

/// `readdir` in the order Node returns it (libuv sorts by byte value on unix; NTFS order on Windows).
pub fn read_dir_names(p: &Path, dirs_only: bool) -> Option<Vec<String>> {
    let mut v: Vec<String> = fs::read_dir(p).ok()?.filter_map(|e| e.ok()).filter(|e| !dirs_only || e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    if !cfg!(windows) { v.sort(); }
    Some(v)
}

static CWD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""cwd":"((?:[^"\\]|\\[^\n\r\x{2028}\x{2029}])*)""#).unwrap());

/// Best-effort real folder for a session: the "cwd" recorded in the first 16 KB of its transcript.
fn read_cwd(file: &Path) -> Option<String> {
    let mut f = fs::File::open(file).ok()?;
    let mut buf = vec![0u8; 16384];
    let mut n = 0;
    while n < buf.len() { match f.read(&mut buf[n..]) { Ok(0) => break, Ok(k) => n += k, Err(_) => return None } }
    let text = String::from_utf8_lossy(&buf[..n]);
    let m = CWD_RE.captures(&text)?;
    match serde_json::from_str::<Value>(&format!("\"{}\"", &m[1])) { Ok(Value::String(s)) => Some(s), _ => None }
}

pub struct Session {
    pub id: String,
    pub main_path: PathBuf,
    pub project_dir: PathBuf,
    pub project_key: String,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub mtime: f64,
    pub tracked: bool,
    pub want_main: bool,
    /// (size, mtime) of the main file when its cwd was last looked for and not found
    cwd_tried: Option<(u64, f64)>,
}

impl Session {
    pub fn project_name(&self) -> &str { self.project.as_deref().filter(|s| !s.is_empty()).unwrap_or(&self.project_key) }
}

pub struct FileRec {
    pub kind: &'static str,
    pub session_id: String,
    pub id: String,
    pub file: PathBuf,
    pub meta_path: Option<PathBuf>,
    offset: u64,
    buf: Vec<u8>,
    meta_mtime: f64,
    pub builder: AgentBuilder,
    pub last_status: Option<&'static str>,
}

pub struct Store {
    pub projects_root: PathBuf,
    pub rules: Vec<Rule>,
    pub stale_sec: f64,
    pub stuck_sec: f64,
    pub sessions: Vec<Session>,
    sess_idx: HashMap<String, usize>,
    pub files: Vec<FileRec>,
    file_idx: HashMap<PathBuf, usize>,
    meta_idx: HashMap<PathBuf, usize>,
    main_idx: HashMap<PathBuf, usize>,
    by_id: HashMap<String, usize>,
    pub last_file_change: f64,
    pub emitted: Vec<Snapshot>,
}

pub struct State {
    pub now: f64,
    pub projects: Value,
    pub selected: Option<String>,
    pub session_ids: Vec<String>,
    pub sessions: Value,
    pub agents: Vec<Snapshot>,
    pub stale_sec: f64,
    pub stuck_sec: f64,
}

impl State {
    pub fn to_value(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("now".into(), Value::from(self.now));
        m.insert("projects".into(), self.projects.clone());
        m.insert("selected".into(), self.selected.as_deref().map(Value::from).unwrap_or(Value::Null));
        m.insert("sessionIds".into(), Value::Array(self.session_ids.iter().map(|s| Value::from(s.as_str())).collect()));
        m.insert("sessions".into(), self.sessions.clone());
        m.insert("agents".into(), Value::Array(self.agents.iter().map(Snapshot::to_value).collect()));
        m.insert("staleSec".into(), Value::from(self.stale_sec));
        m.insert("stuckSec".into(), Value::from(self.stuck_sec));
        m
    }
}

impl Store {
    pub fn new(projects_root: PathBuf, rules: Vec<Rule>) -> Self {
        Store {
            projects_root, rules, stale_sec: STALE_SEC, stuck_sec: STUCK_SEC, sessions: vec![], sess_idx: HashMap::new(),
            files: vec![], file_idx: HashMap::new(), meta_idx: HashMap::new(), main_idx: HashMap::new(), by_id: HashMap::new(), last_file_change: now_ms(), emitted: vec![],
        }
    }

    pub fn opts(&self, now: f64) -> Opts<'_> { Opts { now, stale_sec: self.stale_sec, stuck_sec: self.stuck_sec, rules: &self.rules } }

    pub fn scan(&mut self) {
        let Some(dirs) = read_dir_names(&self.projects_root, true) else { return };
        let now = now_ms();
        for dir_name in dirs {
            let project_dir = self.projects_root.join(&dir_name);
            let Some(names) = read_dir_names(&project_dir, false) else { continue };
            for n in names {
                let Some(id) = n.strip_suffix(".jsonl") else { continue };
                let main_path = project_dir.join(&n);
                let Ok(st) = fs::metadata(&main_path) else { continue };
                let mt = mtime_ms(&st);
                let i = match self.sess_idx.get(id) {
                    Some(&i) => i,
                    None => {
                        self.sessions.push(Session { id: id.to_string(), main_path: main_path.clone(), project_dir: project_dir.clone(), project_key: dir_name.clone(),
                            cwd: None, project: None, mtime: 0.0, tracked: false, want_main: false, cwd_tried: None });
                        self.sess_idx.insert(id.to_string(), self.sessions.len() - 1);
                        self.main_idx.insert(main_path.clone(), self.sessions.len() - 1);
                        self.sessions.len() - 1
                    }
                };
                let s = &mut self.sessions[i];
                s.mtime = s.mtime.max(mt);
                // Node re-reads the head of a cwd-less session on every scan; the answer only changes with the file
                if s.cwd.is_none() && s.cwd_tried != Some((st.len(), mt)) {
                    s.cwd_tried = Some((st.len(), mt));
                    s.cwd = read_cwd(&main_path);
                    if let Some(c) = &s.cwd {
                        let trimmed = c.trim_end_matches(['\\', '/']);
                        let b = crate::parse::basename(trimmed);
                        s.project = Some(if b.is_empty() { c.clone() } else { b });
                    }
                }
                if !s.tracked && now - mt < DAY { s.tracked = true; }
            }
        }
        for i in 0..self.sessions.len() {
            if !self.sessions[i].tracked { continue; }
            self.scan_subagents(i);
            if self.sessions[i].want_main {
                let (p, sid) = (self.sessions[i].main_path.clone(), self.sessions[i].id.clone());
                self.read_file(&p, "main", &sid, &format!("main-{sid}"), None);
            }
        }
    }

    /// Targeted update from file-watcher paths: re-read only the transcripts and metadata that changed. Returns false
    /// when a full scan is needed instead (a new transcript or metadata file, or a session that just became recent).
    /// Directory events are ignored: every new file brings its own event.
    pub fn on_paths(&mut self, paths: &[PathBuf]) -> bool {
        let now = now_ms();
        let mut todo: Vec<usize> = vec![];
        for p in paths {
            let mut known = false;
            if let Some(&si) = self.main_idx.get(p) {
                known = true;
                if let Ok(st) = fs::metadata(p) {
                    let mt = mtime_ms(&st);
                    let s = &mut self.sessions[si];
                    s.mtime = s.mtime.max(mt);
                    if !s.tracked && now - mt < DAY { return false; }
                    if s.cwd.is_none() && s.cwd_tried != Some((st.len(), mt)) { return false; }
                }
            }
            if let Some(&i) = self.file_idx.get(p).or_else(|| self.meta_idx.get(p)) {
                known = true;
                if !todo.contains(&i) { todo.push(i); }
            }
            if !known {
                let name = p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
                if name.ends_with(".jsonl") || name.ends_with(".meta.json") { return false; }
            }
        }
        for i in todo {
            let r = &self.files[i];
            let (f, k, sid, id, mp) = (r.file.clone(), r.kind, r.session_id.clone(), r.id.clone(), r.meta_path.clone());
            if !self.sess_idx.get(&sid).map(|&si| self.sessions[si].tracked).unwrap_or(false) { continue; }
            self.read_file(&f, k, &sid, &id, mp);
        }
        true
    }

    fn scan_subagents(&mut self, si: usize) {
        let dir = self.sessions[si].project_dir.join(&self.sessions[si].id).join("subagents");
        let Some(names) = read_dir_names(&dir, false) else { return };
        let sid = self.sessions[si].id.clone();
        for n in names {
            let Some(id) = n.strip_prefix("agent-").and_then(|x| x.strip_suffix(".jsonl")).filter(|x| !x.is_empty()) else { continue };
            let meta = dir.join(format!("agent-{id}.meta.json"));
            self.read_file(&dir.join(&n), "agent", &sid, id, Some(meta));
        }
    }

    fn read_file(&mut self, file: &Path, kind: &'static str, session_id: &str, id: &str, meta_path: Option<PathBuf>) {
        let ri = match self.file_idx.get(file) {
            Some(&i) => i,
            None => {
                self.files.push(FileRec {
                    kind, session_id: session_id.to_string(), id: id.to_string(), file: file.to_path_buf(), meta_path, offset: 0, buf: vec![],
                    meta_mtime: 0.0, builder: AgentBuilder::new(id, kind, Some(session_id.to_string())), last_status: None,
                });
                let i = self.files.len() - 1;
                self.file_idx.insert(file.to_path_buf(), i);
                if let Some(mp) = &self.files[i].meta_path { self.meta_idx.insert(mp.clone(), i); }
                self.by_id.insert(id.to_string(), i);
                i
            }
        };
        let mut changed = false;
        let rec = &mut self.files[ri];
        if let Some(mp) = &rec.meta_path {
            if let Ok(st) = fs::metadata(mp) {
                let mt = mtime_ms(&st);
                if mt != rec.meta_mtime {
                    rec.meta_mtime = mt;
                    if let Some(v) = fs::read(mp).ok().and_then(|b| js::parse_line(&String::from_utf8_lossy(&b))) {
                        rec.builder.meta = v;
                        changed = true;
                    }
                }
            }
        }
        let Ok(st) = fs::metadata(file) else { return };
        let size = st.len();
        if size < rec.offset {
            let meta = std::mem::take(&mut rec.builder.meta);
            rec.builder = AgentBuilder::new(&rec.id, rec.kind, Some(rec.session_id.clone()));
            rec.builder.meta = meta;
            rec.offset = 0;
            rec.buf.clear();
        }
        if size > rec.offset {
            if let Ok(mut fh) = fs::File::open(file) {
                let len = (size - rec.offset) as usize;
                let mut chunk = vec![0u8; len];
                let mut n = 0;
                if fh.seek(SeekFrom::Start(rec.offset)).is_ok() {
                    while n < len { match fh.read(&mut chunk[n..]) { Ok(0) => break, Ok(k) => n += k, Err(_) => break } }
                }
                rec.offset += n as u64;
                rec.buf.extend_from_slice(&chunk[..n]);
                if let Some(nl) = rec.buf.iter().rposition(|&b| b == b'\n') {
                    let rest = rec.buf.split_off(nl + 1);
                    let text = String::from_utf8_lossy(&rec.buf[..nl]).into_owned();
                    rec.buf = rest;
                    for line in text.split('\n') {
                        if js::trim(line).is_empty() { continue; }
                        if let Some(r) = js::parse_line(line) { if rec.builder.push(&r) { changed = true; } }
                    }
                }
            }
            self.last_file_change = now_ms();
            let sid = rec.session_id.clone();
            let mt = mtime_ms(&st);
            if let Some(&si) = self.sess_idx.get(&sid) { let s = &mut self.sessions[si]; s.mtime = s.mtime.max(mt); }
        }
        if changed { self.emit(ri); }
    }

    fn emit(&mut self, ri: usize) {
        let snap = self.files[ri].builder.snapshot(&self.opts(now_ms()));
        self.files[ri].last_status = Some(snap.status);
        self.emitted.push(snap);
    }

    /// Periodic re-evaluation for time-based status (stale/stuck).
    pub fn tick(&mut self) {
        let now = now_ms();
        for i in 0..self.files.len() {
            let rec = &self.files[i];
            if rec.builder.finished() && rec.last_status == Some("done") { continue; }
            let snap = rec.builder.snapshot(&self.opts(now));
            if Some(snap.status) != self.files[i].last_status {
                self.files[i].last_status = Some(snap.status);
                self.emitted.push(snap);
            }
        }
    }

    pub fn latest_session_id(&self, project: Option<&str>) -> Option<String> {
        let mut best: Option<&Session> = None;
        for s in &self.sessions {
            if let Some(p) = project { if s.project_key != p { continue; } }
            if best.map(|b| s.mtime > b.mtime).unwrap_or(true) { best = Some(s); }
        }
        best.map(|s| s.id.clone())
    }

    fn ensure_tracked(&mut self, id: &str, want_main: bool) {
        let Some(&i) = self.sess_idx.get(id) else { return };
        let s = &mut self.sessions[i];
        let mut need = false;
        if !s.tracked { s.tracked = true; need = true; }
        if want_main && !s.want_main { s.want_main = true; need = true; }
        if need { self.scan(); }
    }

    /// `session` is "latest", "all-recent" or an id; `None`/empty means latest.
    pub fn state(&mut self, session: &str, main: bool, project: Option<&str>) -> State {
        if self.sessions.is_empty() { self.scan(); }
        let now = now_ms();
        let ids: Vec<String> = if session == "all-recent" {
            self.sessions.iter().filter(|s| now - s.mtime < DAY && project.map(|p| s.project_key == p).unwrap_or(true)).map(|s| s.id.clone()).collect()
        } else if session == "latest" || session.is_empty() {
            self.latest_session_id(project).into_iter().collect()
        } else { vec![session.to_string()] };
        for id in &ids { self.ensure_tracked(id, main); }
        let o = self.opts(now);
        let mut agents: Vec<Snapshot> = self.files.iter()
            .filter(|r| ids.contains(&r.session_id) && !(r.kind == "main" && !main))
            .map(|r| r.builder.snapshot(&o)).collect();
        agents.sort_by(|a, b| b.last_event_at.unwrap_or(0.0).partial_cmp(&a.last_event_at.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));

        let mut sess: Vec<&Session> = self.sessions.iter().filter(|s| project.map(|p| s.project_key == p).unwrap_or(true)).collect();
        sess.sort_by(|a, b| b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal));
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for r in self.files.iter().filter(|r| r.kind == "agent") { *counts.entry(r.session_id.as_str()).or_default() += 1; }
        let sessions = Value::Array(sess.iter().take(40).map(|s| {
            let mut m = Map::new();
            m.insert("id".into(), Value::from(s.id.as_str()));
            m.insert("mtime".into(), Value::from(s.mtime));
            m.insert("project".into(), Value::from(s.project_name()));
            m.insert("projectKey".into(), Value::from(s.project_key.as_str()));
            m.insert("cwd".into(), s.cwd.as_deref().map(Value::from).unwrap_or(Value::Null));
            m.insert("agents".into(), Value::from(*counts.get(s.id.as_str()).unwrap_or(&0) as f64));
            m.insert("tracked".into(), Value::from(s.tracked));
            Value::Object(m)
        }).collect());

        // projects: first-seen key order, values from the last session with that key, then sorted by mtime desc
        let mut order: Vec<&str> = vec![];
        let mut last: HashMap<&str, &Session> = HashMap::new();
        let mut pm: HashMap<&str, f64> = HashMap::new();
        for s in &self.sessions {
            if !last.contains_key(s.project_key.as_str()) { order.push(&s.project_key); }
            last.insert(&s.project_key, s);
            let e = pm.entry(&s.project_key).or_insert(0.0);
            *e = e.max(s.mtime);
        }
        let mut projects: Vec<(f64, Value)> = order.iter().map(|k| {
            let s = last[k];
            let mut m = Map::new();
            m.insert("key".into(), Value::from(*k));
            m.insert("name".into(), Value::from(s.project_name()));
            m.insert("cwd".into(), s.cwd.as_deref().map(Value::from).unwrap_or(Value::Null));
            m.insert("mtime".into(), Value::from(pm[k]));
            (pm[k], Value::Object(m))
        }).collect();
        projects.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        State {
            now, projects: Value::Array(projects.into_iter().map(|p| p.1).collect()),
            selected: if session == "all-recent" { Some("all-recent".into()) } else { ids.first().cloned() },
            session_ids: ids, sessions, agents, stale_sec: self.stale_sec, stuck_sec: self.stuck_sec,
        }
    }

    pub fn builder(&self, id: &str) -> Option<&AgentBuilder> { self.by_id.get(id).map(|&i| &self.files[i].builder) }

    /// `/api/agent/:id` variants. `event` is an already-resolved index (None = detail).
    pub fn agent(&self, id: &str, event: Option<f64>, full: bool) -> Option<Value> {
        let b = self.builder(id)?;
        let o = self.opts(now_ms());
        if full { return Some(b.full_detail(&o)); }
        if let Some(ev) = event {
            if ev.fract() != 0.0 || ev < 0.0 || ev.is_nan() { return None; }
            return b.event_value(ev as usize);
        }
        Some(b.detail(&o))
    }

    pub fn session(&self, id: &str) -> Option<&Session> { self.sess_idx.get(id).map(|&i| &self.sessions[i]) }
}

pub fn meta_str(meta: &Value, k: &str) -> String { js::string_or_empty(prop(Some(meta), k)) }
