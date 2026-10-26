//! Kosten des Aktivierungs-Schutzes messen:
//! - Systemton-Mitschnitt: Abmischen + Ringpuffer (läuft dauerhaft, solange etwas abgespielt wird)
//! - Prüfung bei "Panco": 2,5 s Systemton auf das Wake-Word durchsuchen (nur bei einem Treffer)
//!
//! cargo run -p panco-voice --release --example guard_bench
use panco_voice::loopback::Ring;
use panco_voice::wake::{self, WakeDetector};
use std::time::Instant;

fn word(v: f32) -> Vec<f32> {
    (0..16 * 700)
        .map(|i| {
            let t = i as f32 / 16_000.0;
            let f0 = if t < 0.35 { 140.0 + 60.0 * t } else { 210.0 - 80.0 * (t - 0.35) } * (1.0 + v);
            let env = (std::f32::consts::PI * (t / 0.35).fract()).sin().max(0.0);
            env * (1..8).map(|h| (2.0 * std::f32::consts::PI * f0 * h as f32 * t).sin() / h as f32).sum::<f32>() * 0.25
        })
        .collect()
}

fn main() {
    // 1) Mitschnitt: 60 s Stereo-Systemton bei 48 kHz in 10-ms-Blöcken (wie WASAPI)
    let block: Vec<f32> = (0..960).map(|i| (i as f32 * 0.01).sin() * 0.3).collect();
    let mut ring = Ring::new(48_000);
    let t = Instant::now();
    for _ in 0..6000 {
        let mono: Vec<f32> = block.chunks(2).map(|f| f.iter().sum::<f32>() / 2.0).collect();
        ring.push(&mono);
    }
    let per_sec = t.elapsed().as_secs_f64() / 60.0;
    println!(
        "Systemton-Mitschnitt: {:.3} ms Rechenzeit pro Sekunde Audio = {:.3} % eines Kerns",
        per_sec * 1e3,
        per_sec * 100.0
    );

    // 2) Prüfung bei "Panco"
    let path = std::env::temp_dir().join("panco-guard-bench.rpw");
    let samples: Vec<Vec<f32>> = (0..5).map(|i| word(i as f32 * 0.004)).collect();
    wake::train(&samples, 0.5, &path).unwrap();
    let mut det = WakeDetector::load(&path, 0.42).unwrap();
    let t = Instant::now();
    let runs = 20;
    for _ in 0..runs {
        let pcm = ring.last_16k(2_500);
        det.reset();
        let _ = pcm.chunks(480).any(|c| det.process(c).is_some());
    }
    println!("Prüfung bei „Panco“ (2,5 s Systemton): {:.1} ms", t.elapsed().as_secs_f64() * 1e3 / runs as f64);
    let _ = std::fs::remove_file(path);
}
