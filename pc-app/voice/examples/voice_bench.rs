//! Messwerkzeug für die Sprachpipeline ohne Mikrofon.
//!
//! cargo run -p panco-voice --release --example voice_bench -- <audio-ordner> <whisper-modell>
//!
//! Erwartet im Ordner: train_*.wav (Wake-Word "Panco"), cmd_*.wav (Befehle mit "Panco, …"),
//! neg_*.wav (Sätze ohne "Panco"), optional other_*.wav (andere Sprecher).
//! Spielt alles in Echtzeit mit leichtem Rauschen ein und misst Erkennung, CPU und Latenz.

use panco_voice::resample::Resampler;
use panco_voice::{wake, wav, Ctl, Event, Mode, Source, SttEngine, VoiceConfig, VoiceService};
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

fn load16(path: &Path) -> Vec<f32> {
    let (samples, rate) = wav::decode(&std::fs::read(path).unwrap()).expect("16-Bit-WAV");
    let mut out = Vec::new();
    Resampler::new(rate).process(&samples, &mut out);
    out
}

fn files(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(prefix))
        .collect();
    v.sort();
    v
}

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .map(|l| l.split_whitespace().nth(1).unwrap().parse::<f64>().unwrap())
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(&args[1]);
    let model = PathBuf::from(&args[2]);
    let rpw = std::env::temp_dir().join("panco-bench.rpw");

    // 1) Wake-Word aus den Trainingsaufnahmen lernen
    let train: Vec<Vec<f32>> = files(&dir, "train_").iter().map(|p| wake::trim_silence(&load16(p))).collect();
    let t = Instant::now();
    wake::train(&train, 0.5, &rpw).expect("Training");
    println!("Wake-Word trainiert aus {} Aufnahmen in {} ms", train.len(), t.elapsed().as_millis());

    // 2) Pipeline mit eingespeistem Audio starten
    let cfg = VoiceConfig {
        mode: Mode::Wake,
        enabled: true,
        muted: false,
        engine: SttEngine::Whisper,
        wakeword_path: rpw.clone(),
        threshold: 0.5,
        model_path: model,
        keep_model_secs: 3,
        prompt: "Panco, öffne Deadlock. Starte Counter-Strike 2. Lautstärke auf 30 Prozent. Nächstes Lied. Such Build Guides. Fahr den PC herunter. Discord, Spotify, Steam.".into(),
        phrases: vec![],
    };
    let (etx, erx) = channel();
    let voice = VoiceService::start(cfg, Source::Injected { rate: 16_000 }, move |e| {
        let _ = etx.send((Instant::now(), e));
    });
    std::thread::sleep(Duration::from_millis(200));
    let rss_idle = rss_mb();

    // Leises Rauschen (-50 dB) wie ein echtes Mikrofon
    let mut seed = 12345u32;
    let mut noise = |n: usize| -> Vec<f32> {
        (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * 0.003
            })
            .collect()
    };
    let feed = |voice: &VoiceService, audio: &[f32], noise: &mut dyn FnMut(usize) -> Vec<f32>| {
        for chunk in audio.chunks(160) {
            let n = noise(chunk.len());
            voice.inject(chunk.iter().zip(&n).map(|(a, b)| a + b).collect());
            std::thread::sleep(Duration::from_millis(10));
        }
    };

    let mut clips: Vec<(String, Vec<f32>)> = Vec::new();
    for prefix in ["cmd_", "neg_", "other_"] {
        for p in files(&dir, prefix) {
            clips.push((p.file_stem().unwrap().to_string_lossy().into(), load16(&p)));
        }
    }

    // Verstanden? Gleicher Parser wie in Panco.
    let understood = |text: &str| -> String {
        let norm = panco_core::normalize::command(text);
        let intent = panco_core::parser::parse(&norm)
            .or_else(|| norm.split_once(' ').and_then(|(_, r)| panco_core::parser::parse(r)));
        intent.map_or("✗ nicht verstanden".into(), |i| format!("✓ {} {}", i.action, i.params))
    };

    println!("\n{:<26} {:>6} {:>8} {:>8}  Text → Aktion", "Aufnahme", "Wake", "Laden", "Whisper");
    let mut stt_times = Vec::new();
    let mut rss_loaded: f64 = 0.0;
    let mut cpu_samples = Vec::new();
    for (name, audio) in &clips {
        let silence = noise(16_000);
        feed(&voice, &silence, &mut noise);
        let before = voice.stats().wake_cpu_percent();
        cpu_samples.push(before);
        feed(&voice, audio, &mut noise);
        // Stille danach, damit das Befehlsende erkannt wird
        let tail = noise(16_000 * 2);
        feed(&voice, &tail, &mut noise);

        // Ergebnis abwarten (max. 15 s)
        let deadline = Instant::now() + Duration::from_secs(15);
        let (mut woke, mut text, mut load, mut stt) = (String::from("-"), String::new(), 0, 0);
        while let Ok((_, e)) = erx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            match e {
                Event::Wake { score } => woke = format!("{score:.2}"),
                Event::Transcript { transcript, .. } => {
                    text = transcript.text;
                    load = transcript.load_ms;
                    stt = transcript.stt_ms;
                    stt_times.push(stt);
                    rss_loaded = rss_loaded.max(rss_mb());
                    break;
                }
                Event::NoSpeech => {
                    text = "(nichts verstanden)".into();
                    break;
                }
                Event::Error(err) => {
                    text = format!("FEHLER: {err}");
                    break;
                }
                _ => {}
            }
            if woke == "-" && name.starts_with("neg_") && Instant::now() > deadline - Duration::from_secs(14) {
                break;
            }
        }
        let action = if text.is_empty() || text.starts_with('(') {
            String::new()
        } else {
            format!("  → {}", understood(&text))
        };
        println!("{:<26} {:>6} {:>6} ms {:>6} ms  {}{}", name, woke, load, stt, text, action);
    }

    // Leerlauf messen: CPU nur fürs Lauschen über 10 s Rauschen
    let base = voice.stats();
    let (cpu0, audio0) = (
        base.wake_cpu_ns.load(std::sync::atomic::Ordering::Relaxed),
        base.wake_audio_ns.load(std::sync::atomic::Ordering::Relaxed),
    );
    let quiet = noise(16_000 * 10);
    feed(&voice, &quiet, &mut noise);
    let cpu1 = base.wake_cpu_ns.load(std::sync::atomic::Ordering::Relaxed) - cpu0;
    let audio1 = base.wake_audio_ns.load(std::sync::atomic::Ordering::Relaxed) - audio0;
    std::thread::sleep(Duration::from_secs(4)); // Modell wird nach 3 s freigegeben
    let rss_after = rss_mb();

    println!("\nWake-Word-Lauschen, CPU-Anteil eines Kerns:");
    println!("  gesamt (inkl. Sprache):  {:.2} %", voice.stats().wake_cpu_percent());
    println!("  nur Ruhe/Rauschen:       {:.3} %", cpu1 as f64 / audio1.max(1) as f64 * 100.0);
    if !stt_times.is_empty() {
        let avg = stt_times.iter().sum::<u64>() / stt_times.len() as u64;
        println!("Whisper-Erkennung: Ø {avg} ms pro Befehl");
    }
    println!(
        "RAM (RSS): Leerlauf {rss_idle:.1} MB · Whisper geladen {rss_loaded:.1} MB · nach Freigabe {rss_after:.1} MB"
    );
    voice.send(Ctl::Shutdown);
    let _ = std::fs::remove_file(rpw);
}
