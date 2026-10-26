//! Sprecherprüfung bewerten: Profil aus den ersten N Dateien von Sprecher A, dann Abstände für
//! weitere Aufnahmen von A und für andere Sprecher.
//! cargo run -p panco-voice --release --example speaker_eval -- <n> a1.wav … -- b1.wav …
use panco_voice::resample::Resampler;
use panco_voice::speaker::VoiceProfile;
use panco_voice::{wake, wav};

fn load(f: &str) -> Vec<f32> {
    let (s, rate) = wav::decode(&std::fs::read(f).unwrap()).unwrap();
    let mut pcm = Vec::new();
    Resampler::new(rate).process(&s, &mut pcm);
    wake::trim_silence(&pcm)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: usize = args[0].parse().unwrap();
    let split = args.iter().position(|a| a == "--").unwrap();
    let own: Vec<&String> = args[1..split].iter().collect();
    let others: Vec<&String> = args[split + 1..].iter().collect();
    let enroll: Vec<Vec<f32>> = own[..n].iter().map(|f| load(f)).collect();
    let t = std::time::Instant::now();
    let p = VoiceProfile::build(&enroll).unwrap();
    println!("Profil aus {n} Proben ({} µs)", t.elapsed().as_micros());
    let name = |f: &str| std::path::Path::new(f).file_stem().unwrap().to_string_lossy().to_string();
    for (label, files) in [("eigene", &own[n..]), ("fremde", &others[..])] {
        let mut scores: Vec<f32> = files.iter().filter_map(|f| p.score(&load(f))).collect();
        scores.sort_by(f32::total_cmp);
        println!(
            "{label:<7} {} Aufnahmen: {}",
            scores.len(),
            scores.iter().map(|s| format!("{s:.2}")).collect::<Vec<_>>().join(" ")
        );
        let _ = name;
    }
    let pcm = load(own[n]);
    let t = std::time::Instant::now();
    for _ in 0..100 {
        let _ = p.score(&pcm);
    }
    println!("Prüfung: {} µs pro Wake-Word", t.elapsed().as_micros() / 100);
}
