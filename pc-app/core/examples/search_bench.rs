//! Dateisuche messen: cargo run -p panco-core --release --example search_bench -- <ordner> "<anfrage>"
use panco_core::files::search::{parse_query, search, Limits};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = std::path::PathBuf::from(&args[1]);
    let now = panco_core::time::unix_now();
    for query in &args[2..] {
        let q = parse_query(query, now, 3600);
        let t = Instant::now();
        let hits = search(std::slice::from_ref(&root), &[], &q, now, &Limits::default());
        let ms = t.elapsed().as_millis();
        let top: Vec<&str> = hits.iter().take(3).map(|h| h.name.as_str()).collect();
        println!("{query:<34} {ms:>5} ms  {:>4} Treffer  {top:?}", hits.len());
    }
}
