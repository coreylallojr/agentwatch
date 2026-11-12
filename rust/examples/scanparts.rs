//! Where does a poll scan spend its time? cargo run --release --example scanparts
use std::io::Read;
use std::time::Instant;

fn main() {
    let root = agentwatch::sys::default_projects_root();
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;
    for _ in 0..3 {
        let t = Instant::now();
        let mut files = vec![];
        let mut dirs = 0;
        for d in std::fs::read_dir(&root).unwrap().flatten() {
            if d.file_type().unwrap().is_dir() {
                dirs += 1;
                for f in std::fs::read_dir(d.path()).unwrap().flatten() {
                    if f.file_name().to_string_lossy().ends_with(".jsonl") { files.push(f.path()); }
                }
            }
        }
        let a = ms(t);
        let t = Instant::now();
        for f in &files { let _ = std::fs::metadata(f); }
        let b = ms(t);
        let t = Instant::now();
        let mut misses = 0;
        for f in &files {
            let mut fh = std::fs::File::open(f).unwrap();
            let mut buf = vec![0u8; 16384];
            let k = fh.read(&mut buf).unwrap();
            if !String::from_utf8_lossy(&buf[..k]).contains("\"cwd\":\"") { misses += 1; }
        }
        let d = ms(t);
        println!("readdir {dirs} dirs/{} session files {a:.2} ms · fs::metadata on each {b:.2} ms · 16 KB head reads {d:.2} ms ({misses} without cwd)", files.len());
    }
}
