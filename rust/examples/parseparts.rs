//! Cold-scan cost split: file read vs JSON parse vs builder.push (redaction + summaries).
//! cargo run --release --example parseparts -- <projects dir>
use std::time::Instant;

fn walk(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
        let path = e.path();
        if path.is_dir() { walk(&path, out); } else if path.extension().map(|x| x == "jsonl").unwrap_or(false) { out.push(path); }
    }
}

fn main() {
    let root = std::env::args().nth(1).map(std::path::PathBuf::from).unwrap_or_else(agentwatch::sys::default_projects_root);
    let mut files = vec![];
    walk(&root, &mut files);
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let texts: Vec<String> = files.iter().map(|f| String::from_utf8_lossy(&std::fs::read(f).unwrap()).into_owned()).collect();
    let read = ms(t);
    let bytes: usize = texts.iter().map(|t| t.len()).sum();
    let t = Instant::now();
    let recs: Vec<Vec<serde_json::Value>> = texts.iter().map(|t| t.split('\n').filter(|l| !l.trim().is_empty()).filter_map(agentwatch::js::parse_line).collect()).collect();
    let parse = ms(t);
    let t = Instant::now();
    let mut events = 0;
    for r in &recs {
        let mut b = agentwatch::parse::AgentBuilder::new("x", "agent", None);
        for x in r { b.push(x); }
        events += b.events.len();
    }
    let push = ms(t);
    let t = Instant::now();
    let mut n = 0usize;
    for r in &recs { for x in r { n += agentwatch::parse::redact(&x.to_string()).len(); } }
    let red = ms(t);
    println!("{} files, {:.1} MB · read {read:.0} ms · JSON parse {parse:.0} ms · push {push:.0} ms ({events} events) · redact(whole lines) {red:.0} ms ({n})", files.len(), bytes as f64 / 1e6);
}
