//! The `agentwatch` command line (port of bin/agentwatch.mjs plus serve, mcp, setup, install-skills and ideas).
use crate::{clock, js, parse, server, store, sys};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

pub const HELP: &str = "
  agentwatch                open the dashboard (starts the local server if needed), focused on this folder's project
  agentwatch --all          open it with every project visible
  agentwatch status         print active sessions and subagents in the terminal (no server, no browser)
  agentwatch snapshot       print the plain-text collection snapshot (handy for piping to an AI)
  agentwatch stop           stop the background server
  agentwatch --no-open      make sure the server is running but don't open a browser
  agentwatch --port <n>     use another port (default 4778)

  For AI agents:
  agentwatch setup          install the skills and register the MCP server with Claude Code (user scope)
  agentwatch mcp            run the MCP server on stdio (what `claude mcp add` points at)
  agentwatch install-skills copy the agentwatch skills into ~/.claude/skills (or --dir <path>)
  agentwatch ideas          show improvement ideas recorded by agents (~/.agentwatch/ideas.md)
  agentwatch serve          run the dashboard server in the foreground
  agentwatch --version      print the version";

/// Skills shipped inside the binary: (folder, file, contents).
pub const SKILLS: [(&str, &str, &str); 2] = [
    ("agentwatch", "SKILL.md", include_str!("../../skills/agentwatch/SKILL.md")),
    ("agentwatch-evolve", "SKILL.md", include_str!("../../skills/agentwatch-evolve/SKILL.md")),
];

pub fn main(argv: &[String]) -> i32 {
    match argv.first().map(String::as_str) {
        Some("serve") => { serve(&argv[1..]); 0 }
        Some("mcp") => {
            let val = |n: &str| argv.iter().position(|a| a == n).and_then(|i| argv.get(i + 1)).cloned();
            crate::mcp::run(val("--projects").map(PathBuf::from));
            0
        }
        _ => match cli(argv) { Ok(()) => 0, Err(e) => { eprintln!("agentwatch: {e}"); 1 } },
    }
}

// ---------------------------------------------------------------- serve (src/server.mjs main)

fn serve(argv: &[String]) {
    let mut o = server::ServerOpts { exit_on_close: true, ..Default::default() };
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        let peek = argv.get(i + 1).cloned();
        let mut take = || { i += 1; peek.clone().unwrap_or_default() };
        match a {
            "--port" => o.port = js::str_to_number(&take()) as u16,
            "--projects" | "--project" => o.projects = Some(take().into()),
            "--session" => o.session = take(),
            "--no-toast" => o.toast = false,
            "--open" => o.open = Some(match &peek { Some(n) if !n.starts_with("--") => take(), _ => String::new() }),
            "--idle-min" => o.idle_min = js::str_to_number(&take()),
            "--max-idle-h" => o.max_idle_h = js::str_to_number(&take()),
            _ => {}
        }
        i += 1;
    }
    let port = o.port;
    match server::start(o) {
        Ok(h) => {
            eprintln!("Agent Watch on http://127.0.0.1:{}/  (watching all projects)", h.port());
            let app = h.app.clone();
            let _ = ctrlc::set_handler(move || { app.close("signal"); std::process::exit(0); });
            loop { std::thread::sleep(Duration::from_secs(3600)); }
        }
        Err(server::StartError::AddrInUse) => {
            let ours = server::http_get(port, "/api/state", Duration::from_millis(800)).is_some();
            if ours { eprintln!("Agent Watch already running on http://127.0.0.1:{port}/"); } else { eprintln!("Port {port} is busy (another program). Use --port <n>."); }
            std::process::exit(if ours { 0 } else { 3 });
        }
        Err(server::StartError::Other(e)) => { eprintln!("Agent Watch failed: {e}"); std::process::exit(1); }
    }
}

// ---------------------------------------------------------------- dashboard lifecycle

fn pid_file() -> PathBuf { sys::home_dir().join("server.json") }

fn running() -> Option<(u32, u16)> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(pid_file()).ok()?).ok()?;
    let pid = v.get("pid")?.as_f64()? as u32;
    let port = v.get("port")?.as_f64()? as u16;
    if pid != 0 && sys::alive(pid) { Some((pid, port)) } else { None }
}

fn get(port: u16, path: &str) -> Option<String> { server::http_get(port, path, Duration::from_millis(1500)) }

/// The port of a running dashboard server, starting one in the background if needed.
pub fn ensure_server(want: u16) -> Result<u16, String> {
    if let Some((_, port)) = running() { if get(port, "/api/state").is_some() { return Ok(port); } }
    sys::spawn_server(&["--port".into(), want.to_string()]).map_err(|e| e.to_string())?;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(150));
        if get(want, "/api/state").is_some() { return Ok(want); }
    }
    Err(format!("server did not start on port {want} (busy? try --port <n>)"))
}

fn norm(p: &str) -> String { p.replace('\\', "/").trim_end_matches('/').to_lowercase() }

/// The project whose folder is this one (or the closest parent of it).
fn project_for_cwd<'a>(projects: &'a [Value], cwd: &str) -> Option<&'a Value> {
    let enc = parse::project_dir_name(cwd).to_lowercase();
    let c = norm(cwd);
    let key = |p: &Value| p.get("key").and_then(Value::as_str).unwrap_or("").to_string();
    let pcwd = |p: &Value| p.get("cwd").and_then(Value::as_str).map(norm).unwrap_or_default();
    if let Some(p) = projects.iter().find(|p| key(p).to_lowercase() == enc || (p.get("cwd").and_then(Value::as_str).filter(|s| !s.is_empty()).is_some() && pcwd(p) == c)) { return Some(p); }
    let mut best: Option<&Value> = None;
    for p in projects {
        let pc = pcwd(p);
        if !pc.is_empty() && c.starts_with(&format!("{pc}/")) && best.map(|b| pc.len() > pcwd(b).len()).unwrap_or(true) { best = Some(p); }
    }
    best
}

fn ago(ms: f64) -> String {
    let s = js::round(((clock::now_ms() - ms) / 1000.0).max(0.0));
    if s < 90.0 { format!("{}s", js::num(s)) } else if s < 5400.0 { format!("{}m", js::num(js::round(s / 60.0))) } else { format!("{}h", js::num(js::round(s / 3600.0))) }
}

fn cwd() -> String { std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default() }

fn status(all: bool) -> Result<(), String> {
    let mut store = store::Store::new(sys::default_projects_root(), sys::load_rules(None));
    store.scan();
    let st = store.state("all-recent", false, None);
    let projects = st.projects.as_array().cloned().unwrap_or_default();
    let here = project_for_cwd(&projects, &cwd()).cloned();
    let here_key = here.as_ref().and_then(|h| h.get("key")).and_then(Value::as_str).map(str::to_string);
    let now = clock::now_ms();
    let sessions: Vec<&Value> = st.sessions.as_array().map(|a| a.iter().filter(|s| {
        s.get("mtime").and_then(Value::as_f64).unwrap_or(0.0) > now - store::DAY
            && (all || here_key.is_none() || s.get("projectKey").and_then(Value::as_str) == here_key.as_deref())
    }).collect()).unwrap_or_default();
    if sessions.is_empty() {
        let name = here.as_ref().and_then(|h| h.get("name")).map(|v| js::string(Some(v))).unwrap_or_default();
        println!("No Claude Code sessions in the last 24h{}.", if here.is_some() && !all { format!(" for {name} (try --all)") } else { String::new() });
        return Ok(());
    }
    for s in sessions {
        let id = s.get("id").and_then(Value::as_str).unwrap_or("");
        let ag: Vec<&parse::Snapshot> = st.agents.iter().filter(|a| a.session_id.as_deref() == Some(id) && a.kind == "agent").collect();
        let live = ag.iter().filter(|a| a.status != "done").count();
        println!("{}  {}  last activity {} ago  {} subagent{}{}", js::string(s.get("project")), js::slice16(id, 0, 8), ago(s.get("mtime").and_then(Value::as_f64).unwrap_or(0.0)),
            ag.len(), if ag.len() == 1 { "" } else { "s" }, if live > 0 { format!(", {live} active") } else { String::new() });
        for a in ag.iter().take(12) {
            let ty = if js::truthy(Some(&a.ty)) { a.type_str() } else { "agent".into() };
            let d = if js::truthy(Some(&a.description)) { a.desc_str() } else { a.id.clone() };
            println!("   {} {} {}", js::pad_end(a.status, 8), js::pad_end(&ty, 18), js::slice16(&d, 0, 60));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- AI-agent setup

fn claude_skills_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| sys::user_home().join(".claude")).join("skills")
}

/// Write the bundled skills into `dir` (default ~/.claude/skills). Existing files are replaced only when they differ.
pub fn install_skills(dir: Option<PathBuf>) -> Result<Vec<String>, String> {
    let dir = dir.unwrap_or_else(claude_skills_dir);
    let mut done = vec![];
    for (folder, file, text) in SKILLS {
        let d = dir.join(folder);
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        let p = d.join(file);
        let same = std::fs::read_to_string(&p).map(|t| t == text).unwrap_or(false);
        if !same { std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?; }
        done.push(format!("{} {}", if same { "unchanged" } else { "wrote    " }, p.display()));
    }
    Ok(done)
}

fn setup(dry: bool) -> Result<(), String> {
    for l in install_skills(None)? { println!("skill    {l}"); }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_s = exe.to_string_lossy().into_owned();
    let args = ["mcp", "add", "--transport", "stdio", "--scope", "user", "agentwatch", "--", exe_s.as_str(), "mcp"];
    let shown = format!("claude {}", args.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.to_string() }).collect::<Vec<_>>().join(" "));
    if dry { println!("mcp      would run: {shown}"); return Ok(()); }
    let claude = if cfg!(windows) { "claude.cmd" } else { "claude" };
    let run = |bin: &str| std::process::Command::new(bin).args(args).output();
    match run(claude).or_else(|_| run("claude")) {
        Ok(o) if o.status.success() => println!("mcp      registered: {shown}"),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if err.contains("already exists") { println!("mcp      already registered (remove with `claude mcp remove agentwatch --scope user` to re-add)"); }
            else { println!("mcp      `claude mcp add` failed ({err}). Run it yourself:\n         {shown}"); }
        }
        Err(_) => println!("mcp      Claude Code CLI not found on PATH. Register the server yourself:\n         {shown}"),
    }
    println!("\nRestart Claude Code (or run /mcp) to load the agentwatch tools. Skills are picked up automatically.");
    Ok(())
}

fn ideas() -> Result<(), String> {
    let p = sys::home_dir().join("ideas.md");
    match std::fs::read_to_string(&p) {
        Ok(t) => { println!("{}\n\n{}", p.display(), t.trim_end()); }
        Err(_) => println!("No ideas recorded yet ({} will appear when an agent calls agentwatch_suggest_improvement).", p.display()),
    }
    Ok(())
}

fn cli(argv: &[String]) -> Result<(), String> {
    let flag = |n: &str| argv.iter().any(|a| a == n);
    let val = |n: &str| argv.iter().position(|a| a == n).and_then(|i| argv.get(i + 1)).cloned();
    let cmd = argv.iter().enumerate().find(|(i, a)| !a.starts_with('-') && (*i == 0 || argv[i - 1] != "--port")).map(|(_, a)| a.as_str()).unwrap_or("open");
    if flag("-V") || flag("--version") || cmd == "version" { println!("agentwatch {}", env!("CARGO_PKG_VERSION")); return Ok(()); }
    if flag("-h") || flag("--help") || cmd == "help" { println!("{HELP}"); return Ok(()); }
    match cmd {
        "status" => return status(flag("--all") || flag("-a")),
        "setup" => return setup(flag("--dry-run")),
        "install-skills" => { for l in install_skills(val("--dir").map(PathBuf::from))? { println!("{l}"); } return Ok(()); }
        "ideas" => return ideas(),
        "stop" => {
            return match running() {
                None => { println!("Agent Watch is not running."); Ok(()) }
                Some((pid, _)) => { sys::kill(pid)?; println!("Stopped Agent Watch (pid {pid})."); Ok(()) }
            };
        }
        _ => {}
    }
    let want = val("--port").map(|p| js::str_to_number(&p)).filter(|n| *n > 0.0).map(|n| n as u16).unwrap_or(sys::DEFAULT_PORT);
    let port = ensure_server(want)?;
    if cmd == "snapshot" {
        let q = val("--session").map(|s| format!("?session={s}")).unwrap_or_default();
        print!("{}", get(port, &format!("/api/collection/snapshot.txt{q}")).filter(|s| !s.is_empty()).unwrap_or_else(|| "no data\n".into()));
        return Ok(());
    }
    let mut q = String::new();
    if !flag("--all") {
        let state: Value = get(port, "/api/state?session=all-recent").and_then(|b| serde_json::from_str(&b).ok()).unwrap_or(Value::Null);
        let projects = state.get("projects").and_then(Value::as_array).cloned().unwrap_or_default();
        if let Some(p) = project_for_cwd(&projects, &cwd()) { q = format!("?project={}", js::encode_uri_component(&js::string(p.get("key")))); }
    }
    let url = format!("http://127.0.0.1:{port}/{q}");
    if !flag("--no-open") { sys::open_browser(&url); }
    println!("Agent Watch: {url}");
    Ok(())
}
