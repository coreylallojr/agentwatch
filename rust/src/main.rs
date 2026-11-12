//! Entry point. Everything lives in the library (src/lib.rs); this file adds two developer-only commands:
//! `__dump` (parity harness, see parity/run.mjs) and `__profile` (in-process timings).
use agentwatch::{cli, clock, collection, js, store, sys};

use serde_json::Value;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match argv.first().map(String::as_str) {
        Some("__dump") => dump(&argv[1..]),
        Some("__profile") => profile(&argv[1..]),
        _ => std::process::exit(cli::main(&argv)),
    }
}

// ---------------------------------------------------------------- parity dump

/// `agentwatch __dump --projects <dir> --now <ms>`: every API surface for every session, as one JSON document, with
/// a pinned clock. parity/oracle.mjs produces the same document from the Node implementation.
fn dump(argv: &[String]) {
    let val = |n: &str| argv.iter().position(|a| a == n).and_then(|i| argv.get(i + 1)).cloned();
    let now = js::str_to_number(&val("--now").unwrap_or_default());
    clock::set_fake_now(now);
    let root = std::path::PathBuf::from(val("--projects").expect("--projects"));
    let rules = sys::compile_text_pub(sys::BUNDLED_RULES);
    let mut st = store::Store::new(root, rules);
    st.scan();
    let mut out = serde_json::Map::new();
    let mut add = |k: String, v: Value| { out.insert(k, v); };
    let projects: Vec<String> = st.state("all-recent", false, None).projects.as_array().unwrap().iter().map(|p| js::string(p.get("key"))).collect();
    let mut scopes: Vec<(String, Option<String>)> = vec![("latest".into(), None), ("all-recent".into(), None)];
    for p in &projects { scopes.push(("latest".into(), Some(p.clone()))); scopes.push(("all-recent".into(), Some(p.clone()))); }
    let sids: Vec<String> = st.sessions.iter().map(|s| s.id.clone()).collect();
    for s in &sids { scopes.push((s.clone(), None)); }
    for (sess, proj) in &scopes {
        for main in [false, true] {
            let v = st.state(sess, main, proj.as_deref()).to_value();
            add(format!("state|{sess}|{}|{main}", proj.clone().unwrap_or_default()), Value::from(js::json(&Value::Object(v))));
        }
        let s2 = st.state(sess, true, proj.as_deref());
        let m = collection::build_model(s2, |id| st.builder(id), now);
        let key = format!("{sess}|{}", proj.clone().unwrap_or_default());
        add(format!("page|{key}"), Value::from(collection::render_page(&m)));
        add(format!("fragment|{key}"), Value::from(collection::render_body(&m)));
        add(format!("snapshot|{key}"), Value::from(collection::snapshot_text(&m)));
    }
    let ids: Vec<(String, usize)> = st.files.iter().map(|f| (f.id.clone(), f.builder.events.len())).collect();
    for (id, n) in ids {
        let j = |v: Option<Value>| Value::from(js::json(&v.unwrap_or(Value::Null)));
        add(format!("agent|{id}"), j(st.agent(&id, None, false)));
        add(format!("agentfull|{id}"), j(st.agent(&id, None, true)));
        for i in 0..n { add(format!("event|{id}|{i}"), j(st.agent(&id, Some(i as f64), false))); }
    }
    print!("{}", js::json(&Value::Object(out)));
}

// ---------------------------------------------------------------- profiling

/// `agentwatch __profile [--projects <dir>]`: in-process timings of the hot paths (no HTTP).
fn profile(argv: &[String]) {
    use std::time::Instant;
    let val = |n: &str| argv.iter().position(|a| a == n).and_then(|i| argv.get(i + 1)).cloned();
    let root = val("--projects").map(std::path::PathBuf::from).unwrap_or_else(sys::default_projects_root);
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;
    let mut st = store::Store::new(root, sys::load_rules(None));
    let t = Instant::now(); st.scan(); println!("cold scan            {:8.1} ms  ({} sessions, {} files)", ms(t), st.sessions.len(), st.files.len());
    let t = Instant::now(); for _ in 0..20 { st.scan(); } println!("warm scan (avg of 20){:8.1} ms", ms(t) / 20.0);
    let t = Instant::now(); let s = st.state("all-recent", true, None); println!("state all-recent+main{:8.1} ms  ({} agents, first call: tracks + reads main files)", ms(t), s.agents.len());
    let t = Instant::now(); for _ in 0..20 { st.state("all-recent", true, None); } println!("state again (avg)    {:8.1} ms", ms(t) / 20.0);
    let t = Instant::now(); for _ in 0..20 { st.scan(); } println!("warm scan, mains on  {:8.1} ms", ms(t) / 20.0);
    let t = Instant::now(); for _ in 0..20 { st.tick(); } println!("tick (avg)           {:8.2} ms", ms(t) / 20.0);
    let s = st.state("all-recent", true, None);
    let now = clock::now_ms();
    let s_json = js::json(&serde_json::Value::Object(s.to_value())).len();
    let t = Instant::now(); let m = collection::build_model(s, |id| st.builder(id), now); println!("build_model          {:8.1} ms  ({} events)", ms(t), m.total_events);
    let t = Instant::now(); let p = collection::render_page(&m); println!("render_page          {:8.1} ms  ({} KB)", ms(t), p.len() / 1024);
    let t = Instant::now(); let x = collection::snapshot_text(&m); println!("snapshot_text        {:8.1} ms  ({} KB)", ms(t), x.len() / 1024);
    println!("state JSON size      {:8} KB", s_json / 1024);
    if let Some(big) = st.files.iter().max_by_key(|f| f.builder.events.len()) {
        let id = big.id.clone();
        let t = Instant::now(); let d = st.agent(&id, None, true).unwrap(); let j = js::json(&d); println!("agent full JSON      {:8.1} ms  ({} KB, {} events)", ms(t), j.len() / 1024, big.builder.events.len());
    }
}
