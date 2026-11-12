//! Port of startServer() in src/server.mjs: 127.0.0.1-only HTTP server, SSE broadcast, scan/tick/ping/idle timers,
//! focus-rule notifications, pidfile. Thread-per-connection on std; no async runtime.
use crate::clock::now_ms;
use crate::collection;
use crate::js;
use crate::parse::{Rule, Snapshot};
use crate::store::Store;
use crate::sys;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const INDEX_HTML: &str = include_str!("../../src/index.html");

pub struct ServerOpts {
    pub port: u16,
    pub projects: Option<PathBuf>,
    pub session: String,
    pub toast: bool,
    pub open: Option<String>,
    pub idle_min: f64,
    pub max_idle_h: f64,
    pub pidfile: bool,
    pub root: PathBuf,
    pub rules: Option<Vec<Rule>>,
    pub exit_on_close: bool,
}

impl Default for ServerOpts {
    fn default() -> Self {
        ServerOpts { port: sys::DEFAULT_PORT, projects: None, session: "latest".into(), toast: true, open: None, idle_min: 30.0, max_idle_h: 8.0,
            pidfile: true, root: sys::home_dir(), rules: None, exit_on_close: false }
    }
}

struct Client { id: u64, stream: TcpStream }

pub struct App {
    pub store: Mutex<Store>,
    clients: Mutex<Vec<Client>>,
    next_client: Mutex<u64>,
    pub started_at: f64,
    last_client_at: Mutex<f64>,
    toast_log: Mutex<HashMap<String, f64>>,
    seen_focus: Mutex<HashSet<String>>,
    opts: ServerOpts,
    closed: AtomicBool,
    pid_path: PathBuf,
    pub port: u16,
    scan_tx: Mutex<Option<mpsc::Sender<Option<Vec<PathBuf>>>>>,
}

pub struct Handle { pub app: Arc<App> }

impl Handle {
    pub fn port(&self) -> u16 { self.app.port }
    pub fn close(&self) { self.app.close("close") }
}

fn chunk(data: &str) -> Vec<u8> { format!("{:x}\r\n{data}\r\n", data.len()).into_bytes() }

impl App {
    fn broadcast(&self, obj: &Value) {
        let data = chunk(&format!("data: {}\n\n", js::json(obj)));
        let mut cs = self.clients.lock().unwrap();
        cs.retain_mut(|c| c.stream.write_all(&data).is_ok());
    }

    /// Node's `onAgent` callback.
    fn on_agent(&self, agent: &Snapshot) {
        let mut m = Map::new();
        m.insert("type".into(), Value::from("agent"));
        m.insert("agent".into(), agent.to_value());
        self.broadcast(&Value::Object(m));
        for h in &agent.focus {
            let key = format!("{}|{}|{}", agent.id, h.rule_str(), h.idx);
            {
                let mut seen = self.seen_focus.lock().unwrap();
                if seen.contains(&key) { continue; }
                seen.insert(key);
            }
            let fresh = if h.rule.as_ref().and_then(Value::as_str) == Some("stuck") || h.idx == -1 {
                agent.last_event_at.unwrap_or(0.0) > self.started_at - 60.0 * 60.0 * 1000.0
            } else { h.t.unwrap_or(0.0) >= self.started_at - 10_000.0 };
            if !fresh { continue; }
            let mut f = Map::new();
            f.insert("type".into(), Value::from("focus"));
            f.insert("agentId".into(), Value::from(agent.id.as_str()));
            f.insert("sessionId".into(), agent.session_id.as_deref().map(Value::from).unwrap_or(Value::Null));
            if let Some(r) = &h.rule { f.insert("rule".into(), r.clone()); }
            f.insert("action".into(), h.action.clone());
            f.insert("summary".into(), Value::from(h.summary.as_str()));
            f.insert("t".into(), h.t.map(Value::from).unwrap_or(Value::Null));
            self.broadcast(&Value::Object(f));
            if h.action.as_str() == Some("toast") && self.opts.toast {
                let tk = format!("{}|{}", agent.id, h.rule_str());
                let now = now_ms();
                let mut log = self.toast_log.lock().unwrap();
                if now - log.get(&tk).copied().unwrap_or(0.0) < 5.0 * 60.0 * 1000.0 { continue; }
                log.insert(tk, now);
                let desc = if js::truthy(Some(&agent.description)) { agent.desc_str() } else { agent.id.clone() };
                sys::show_toast(&format!("Agent Watch: {}", h.rule_str()), &format!("{} - {desc}", agent.type_str()));
            }
        }
    }

    /// Run `f` on the store, then deliver whatever it emitted (outside the lock, in order).
    pub fn with_store<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R {
        let (r, emitted) = {
            let mut s = self.store.lock().unwrap();
            let r = f(&mut s);
            (r, std::mem::take(&mut s.emitted))
        };
        for a in &emitted { self.on_agent(a); }
        r
    }

    pub fn close(&self, _reason: &str) {
        if self.closed.swap(true, Ordering::SeqCst) { return; }
        for mut c in self.clients.lock().unwrap().drain(..) { let _ = c.stream.write_all(b"0\r\n\r\n"); let _ = c.stream.shutdown(Shutdown::Both); }
        *self.scan_tx.lock().unwrap() = None;
        if self.opts.pidfile {
            if let Ok(t) = std::fs::read_to_string(&self.pid_path) {
                if let Ok(v) = serde_json::from_str::<Value>(&t) {
                    if v.get("pid").and_then(Value::as_f64) == Some(std::process::id() as f64) { let _ = std::fs::remove_file(&self.pid_path); }
                }
            }
        }
    }

    pub fn is_closed(&self) -> bool { self.closed.load(Ordering::SeqCst) }
}

// ---------------------------------------------------------------- HTTP

struct Resp { status: u16, ctype: Option<&'static str>, cache: Option<&'static str>, body: Vec<u8> }

fn reason(code: u16) -> &'static str {
    match code { 200 => "OK", 403 => "Forbidden", 404 => "Not Found", 405 => "Method Not Allowed", 500 => "Internal Server Error", _ => "" }
}

fn json_resp(code: u16, v: &Value) -> Resp { Resp { status: code, ctype: Some("application/json; charset=utf-8"), cache: Some("no-store"), body: js::json(v).into_bytes() } }
fn not_found() -> Resp { json_resp(404, &serde_json::json!({ "error": "not found" })) }

fn write_resp(s: &mut TcpStream, r: &Resp, head: bool, keep: bool) -> std::io::Result<()> {
    let mut h = format!("HTTP/1.1 {} {}\r\n", r.status, reason(r.status));
    if let Some(c) = r.ctype { h.push_str(&format!("content-type: {c}\r\n")); }
    if let Some(c) = r.cache { h.push_str(&format!("cache-control: {c}\r\n")); }
    h.push_str(&format!("Content-Length: {}\r\nConnection: {}\r\n\r\n", r.body.len(), if keep { "keep-alive" } else { "close" }));
    s.write_all(h.as_bytes())?;
    if !head { s.write_all(&r.body)?; }
    s.flush()
}

fn host_ok(host: &str) -> bool {
    let (h, port) = match host.rfind(':') { Some(i) if !host[..i].contains(':') => (&host[..i], Some(&host[i + 1..])), _ => (host, None) };
    let hl = h.to_ascii_lowercase();
    (hl == "127.0.0.1" || hl == "localhost") && port.map(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).unwrap_or(true)
}

fn route(app: &App, path: &str, q: &url::Url) -> Result<Resp, String> {
    let param = |k: &str| q.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
    let session = param("session").filter(|s| !s.is_empty()).unwrap_or_else(|| app.opts.session.clone());
    let project = param("project").filter(|s| !s.is_empty());
    if path == "/" || path == "/index.html" {
        return Ok(Resp { status: 200, ctype: Some("text/html; charset=utf-8"), cache: Some("no-store"), body: INDEX_HTML.as_bytes().to_vec() });
    }
    if path == "/collection" || path == "/api/collection/snapshot.txt" {
        // Rendered while holding the store lock: the model borrows the stored events instead of copying them.
        let txt = path.ends_with(".txt");
        let fragment = param("fragment").as_deref() == Some("1");
        let body = app.with_store(|s| {
            let st = s.state(&session, true, project.as_deref());
            let now = now_ms();
            let m = collection::build_model(st, |id| s.builder(id), now);
            if txt { collection::snapshot_text(&m) } else if fragment { collection::render_body(&m) } else { collection::render_page(&m) }
        });
        let ctype = if txt { "text/plain; charset=utf-8" } else { "text/html; charset=utf-8" };
        return Ok(Resp { status: 200, ctype: Some(ctype), cache: Some("no-store"), body: body.into_bytes() });
    }
    if let Some(rest) = path.strip_prefix("/api/image/") {
        let parts: Vec<String> = rest.split('/').map(js::decode_uri_component).collect::<Result<_, _>>()?;
        let id = parts.first().cloned().unwrap_or_default();
        let ev = js::str_to_number(parts.get(1).map(String::as_str).unwrap_or("\u{0}"));
        let k = parts.get(2).map(|s| js::str_to_number(s)).unwrap_or(f64::NAN);
        let im = app.with_store(|s| s.agent(&id, Some(ev), false))
            .and_then(|v| v.get("event")?.get("images")?.as_array()?.get(if k.fract() == 0.0 && k >= 0.0 { k as usize } else { usize::MAX }).cloned());
        let Some(im) = im else { return Ok(not_found()) };
        let data = im.get("data").and_then(Value::as_str).unwrap_or("");
        if data.is_empty() { return Ok(not_found()); }
        let mt = im.get("mediaType").and_then(Value::as_str).unwrap_or("");
        let ctype: &'static str = match mt { "image/png" => "image/png", "image/jpeg" => "image/jpeg", "image/gif" => "image/gif", "image/webp" => "image/webp", _ => "application/octet-stream" };
        return Ok(Resp { status: 200, ctype: Some(ctype), cache: Some("max-age=3600"), body: js::base64_decode(data) });
    }
    if path == "/api/state" {
        let main = param("main").as_deref() == Some("1");
        let mut v = app.with_store(|s| s.state(&session, main, project.as_deref()).to_value());
        let mut srv = Map::new();
        srv.insert("startedAt".into(), Value::from(app.started_at));
        srv.insert("pid".into(), Value::from(std::process::id() as f64));
        v.insert("server".into(), Value::Object(srv));
        return Ok(json_resp(200, &Value::Object(v)));
    }
    if let Some(rest) = path.strip_prefix("/api/agent/") {
        let id = js::decode_uri_component(rest)?;
        let ev = param("event").map(|e| js::str_to_number(&e));
        let full = param("full").as_deref() == Some("1");
        let body = app.with_store(|s| s.agent(&id, ev, full));
        return Ok(match body { Some(b) => json_resp(200, &b), None => not_found() });
    }
    Ok(not_found())
}

fn handle_conn(app: Arc<App>, mut s: TcpStream) {
    sys::set_thread_qos(true);
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = s.set_nodelay(true);
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    loop {
        // read one request head
        let (method, target, host, keep, consumed) = loop {
            let mut headers = [httparse::EMPTY_HEADER; 64];
            let mut req = httparse::Request::new(&mut headers);
            match req.parse(&buf) {
                Ok(httparse::Status::Complete(n)) => {
                    let host = req.headers.iter().find(|h| h.name.eq_ignore_ascii_case("host")).map(|h| String::from_utf8_lossy(h.value).into_owned()).unwrap_or_default();
                    let conn = req.headers.iter().find(|h| h.name.eq_ignore_ascii_case("connection")).map(|h| String::from_utf8_lossy(h.value).to_ascii_lowercase()).unwrap_or_default();
                    let keep = if req.version == Some(0) { conn.contains("keep-alive") } else { !conn.contains("close") };
                    let has_body = req.headers.iter().any(|h| (h.name.eq_ignore_ascii_case("content-length") && h.value != b"0") || h.name.eq_ignore_ascii_case("transfer-encoding"));
                    break (req.method.unwrap_or("").to_string(), req.path.unwrap_or("/").to_string(), host, keep && !has_body, n);
                }
                Ok(httparse::Status::Partial) => {
                    if buf.len() > 64 * 1024 { return; }
                    let mut tmp = [0u8; 4096];
                    match s.read(&mut tmp) { Ok(0) | Err(_) => return, Ok(n) => buf.extend_from_slice(&tmp[..n]) }
                }
                Err(_) => { let _ = s.write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n"); return; }
            }
        };
        buf.drain(..consumed);
        let head = method == "HEAD";
        let resp = if !host_ok(&host) {
            Resp { status: 403, ctype: None, cache: None, body: b"forbidden".to_vec() }
        } else if method != "GET" && method != "HEAD" {
            Resp { status: 405, ctype: None, cache: None, body: vec![] }
        } else {
            let base = url::Url::parse("http://127.0.0.1").unwrap();
            match base.join(&target) {
                Err(e) => Resp { status: 500, ctype: None, cache: None, body: e.to_string().into_bytes() },
                Ok(u) if u.path() == "/events" => {
                    // SSE: hand the socket to the broadcaster and wait for the client to go away
                    let hello = format!("retry: 2000\ndata: {}\n\n", js::json(&serde_json::json!({ "type": "hello", "startedAt": app.started_at })));
                    let mut h = String::from("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-store\r\nconnection: keep-alive\r\nTransfer-Encoding: chunked\r\n\r\n");
                    if head { let _ = s.write_all(h.as_bytes()); return; }
                    h.push_str(std::str::from_utf8(&chunk(&hello)).unwrap());
                    if s.write_all(h.as_bytes()).is_err() { return; }
                    let Ok(w) = s.try_clone() else { return };
                    let id = { let mut n = app.next_client.lock().unwrap(); *n += 1; *n };
                    app.clients.lock().unwrap().push(Client { id, stream: w });
                    *app.last_client_at.lock().unwrap() = now_ms();
                    let _ = s.set_read_timeout(None);
                    let mut tmp = [0u8; 512];
                    while let Ok(n) = s.read(&mut tmp) { if n == 0 { break; } }
                    app.clients.lock().unwrap().retain(|c| c.id != id);
                    *app.last_client_at.lock().unwrap() = now_ms();
                    return;
                }
                Ok(u) => {
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| route(&app, u.path(), &u)));
                    match r {
                        Ok(Ok(r)) => r,
                        Ok(Err(msg)) => Resp { status: 500, ctype: None, cache: None, body: msg.into_bytes() },
                        Err(_) => Resp { status: 500, ctype: None, cache: None, body: b"internal error".to_vec() },
                    }
                }
            }
        };
        if write_resp(&mut s, &resp, head, keep).is_err() || !keep { let _ = s.shutdown(Shutdown::Both); return; }
    }
}

// ---------------------------------------------------------------- lifecycle

pub enum StartError { AddrInUse, Other(String) }

pub fn start(opts: ServerOpts) -> Result<Handle, StartError> {
    let projects_root = match &opts.projects { Some(p) => std::path::absolute(p).unwrap_or(p.clone()), None => sys::default_projects_root() };
    let listener = TcpListener::bind(("127.0.0.1", opts.port)).map_err(|e| if e.kind() == std::io::ErrorKind::AddrInUse { StartError::AddrInUse } else { StartError::Other(e.to_string()) })?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(opts.port);
    let rules = opts.rules.clone().unwrap_or_else(|| sys::load_rules(None));
    let started_at = now_ms();
    let pid_path = opts.root.join("server.json");
    let app = Arc::new(App {
        store: Mutex::new(Store::new(projects_root.clone(), rules)), clients: Mutex::new(vec![]), next_client: Mutex::new(0), started_at,
        last_client_at: Mutex::new(started_at), toast_log: Mutex::new(HashMap::new()), seen_focus: Mutex::new(HashSet::new()),
        closed: AtomicBool::new(false), pid_path, port, scan_tx: Mutex::new(None), opts,
    });

    // accept loop (Node starts listening before the first scan; requests block on the store until it finishes)
    {
        let app = app.clone();
        std::thread::spawn(move || {
            for s in listener.incoming() {
                if app.is_closed() { break; }
                if let Ok(s) = s { let app = app.clone(); std::thread::spawn(move || handle_conn(app, s)); }
            }
        });
    }
    app.with_store(|s| s.scan());

    // scanner. With a working watcher: changed paths are re-read directly (60 ms quiet-period debounce, but never
    // more than 200 ms behind under a continuous write stream) and a full scan runs every 5 s as a safety net.
    // Without a watcher: a full scan every 1 s, as in the Node version. Status tick every 5 s.
    let (tx, rx) = mpsc::channel::<Option<Vec<PathBuf>>>();
    *app.scan_tx.lock().unwrap() = Some(tx.clone());
    let watcher = {
        let tx = tx.clone();
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let _ = tx.send(match res { Ok(ev) if !ev.need_rescan() && !ev.paths.is_empty() => Some(ev.paths), _ => None });
        }).ok().and_then(|mut w| {
            use notify::Watcher;
            w.watch(&projects_root, notify::RecursiveMode::Recursive).ok().map(|_| w)
        })
    };
    drop(tx);
    let poll_every = if watcher.is_some() { Duration::from_secs(5) } else { Duration::from_secs(1) };
    {
        let app = app.clone();
        std::thread::spawn(move || {
            let _watcher = watcher; // keep alive for the life of the scanner
            sys::set_thread_qos(false);
            let mut next_poll = Instant::now() + poll_every;
            let mut next_tick = Instant::now() + Duration::from_secs(5);
            let mut quiet: Option<Instant> = None;
            let mut deadline: Option<Instant> = None;
            let mut pending: Vec<PathBuf> = vec![];
            let mut full = false;
            loop {
                if app.is_closed() { break; }
                let now = Instant::now();
                let mut wake = next_poll.min(next_tick);
                if let (Some(q), Some(d)) = (quiet, deadline) { wake = wake.min(q.min(d)); }
                match rx.recv_timeout(wake.saturating_duration_since(now)) {
                    Ok(paths) => {
                        match paths { Some(p) => { for x in p { if !pending.contains(&x) { pending.push(x); } } } None => full = true }
                        let t = Instant::now();
                        quiet = Some(t + Duration::from_millis(60));
                        deadline.get_or_insert(t + Duration::from_millis(200));
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) if app.is_closed() => break,
                    Err(_) => {}
                }
                let now = Instant::now();
                if quiet.is_some() && (quiet.map(|q| now >= q).unwrap_or(false) || deadline.map(|d| now >= d).unwrap_or(false)) {
                    quiet = None; deadline = None;
                    let paths = std::mem::take(&mut pending);
                    let need_full = std::mem::take(&mut full);
                    app.with_store(|s| { if need_full || !s.on_paths(&paths) { s.scan(); } });
                }
                if now >= next_poll { next_poll = now + poll_every; app.with_store(|s| s.scan()); }
                if now >= next_tick { next_tick = now + Duration::from_secs(5); app.with_store(|s| s.tick()); }
            }
        });
    }
    // ping (15 s) and idle exit (60 s)
    {
        let app = app.clone();
        std::thread::spawn(move || {
            sys::set_thread_qos(false);
            let mut n = 0u64;
            loop {
                std::thread::sleep(Duration::from_secs(15));
                if app.is_closed() { break; }
                {
                    let ping = chunk(": ping\n\n");
                    let mut cs = app.clients.lock().unwrap();
                    cs.retain_mut(|c| c.stream.write_all(&ping).is_ok());
                    if !cs.is_empty() { *app.last_client_at.lock().unwrap() = now_ms(); }
                }
                n += 1;
                if n % 4 == 0 {
                    let now = now_ms();
                    let quiet = now - app.store.lock().unwrap().last_file_change;
                    let nclients = app.clients.lock().unwrap().len();
                    let idle_ms = app.opts.idle_min * 60_000.0;
                    if nclients == 0 && now - *app.last_client_at.lock().unwrap() >= idle_ms && quiet >= idle_ms { app.close("idle"); if app.opts.exit_on_close { std::process::exit(0); } }
                    else if quiet >= app.opts.max_idle_h * 3_600_000.0 { app.close("idle-max"); if app.opts.exit_on_close { std::process::exit(0); } }
                }
            }
        });
    }

    if app.opts.pidfile {
        let _ = std::fs::create_dir_all(&app.opts.root);
        let mut v = Map::new();
        v.insert("pid".into(), Value::from(std::process::id() as f64));
        v.insert("port".into(), Value::from(port as f64));
        v.insert("startedAt".into(), Value::from(js::iso(started_at)));
        let _ = std::fs::write(&app.pid_path, js::json_pretty(&Value::Object(v)));
    }
    if let Some(o) = &app.opts.open {
        let hash = if o.is_empty() { String::new() } else if o.starts_with('#') { o.clone() } else { format!("#{o}") };
        sys::open_browser(&format!("http://127.0.0.1:{port}/{hash}"));
    }
    Ok(Handle { app })
}

/// GET http://127.0.0.1:port{path}; Some(body) on 200.
pub fn http_get(port: u16, path: &str, timeout: Duration) -> Option<String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, timeout).ok()?;
    s.set_read_timeout(Some(timeout)).ok()?;
    s.set_write_timeout(Some(timeout)).ok()?;
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes()).ok()?;
    let mut raw = vec![];
    s.read_to_end(&mut raw).ok()?;
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut resp = httparse::Response::new(&mut headers);
    let n = match resp.parse(&raw).ok()? { httparse::Status::Complete(n) => n, _ => return None };
    if resp.code != Some(200) { return None; }
    let chunked = resp.headers.iter().any(|h| h.name.eq_ignore_ascii_case("transfer-encoding"));
    let body = &raw[n..];
    let bytes = if chunked { dechunk(body) } else { body.to_vec() };
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    while let Some(i) = b.windows(2).position(|w| w == b"\r\n") {
        let n = usize::from_str_radix(std::str::from_utf8(&b[..i]).unwrap_or("0").trim(), 16).unwrap_or(0);
        if n == 0 || i + 2 + n > b.len() { break; }
        out.extend_from_slice(&b[i + 2..i + 2 + n]);
        b = &b[(i + 4 + n).min(b.len())..];
    }
    out
}
