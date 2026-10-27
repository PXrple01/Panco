//! Pipeline-Tests mit eingespeistem Audio (kein Mikrofon, kein Whisper-Modell nötig).

use panco_voice::{Ctl, Event, MicState, Mode, Source, SttEngine, VoiceConfig, VoiceService};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("panco-voice-{}-{name}", std::process::id()))
}

fn config(mode: Mode) -> VoiceConfig {
    VoiceConfig {
        mode,
        enabled: true,
        muted: false,
        engine: SttEngine::Whisper,
        wakeword_path: tmp("fehlt.rpw"),
        threshold: 0.5,
        model_path: tmp("fehlt.bin"),
        keep_model_secs: 5,
        quality: Default::default(),
        prompt: String::new(),
        phrases: vec![],
        loopback_check: true,
        speaker_check: false,
        profile_path: tmp("stimmprofil.json"),
    }
}

fn start(cfg: VoiceConfig) -> (VoiceService, Receiver<Event>) {
    let (tx, rx) = channel();
    let svc = VoiceService::start(cfg, Source::Injected { rate: 16_000 }, move |e| {
        let _ = tx.send(e);
    });
    (svc, rx)
}

/// "Sprache": Ton mit wechselnder Frequenz, laut genug für die VAD.
fn speech(ms: usize, seed: f32) -> Vec<f32> {
    (0..16 * ms)
        .map(|i| {
            let t = i as f32 / 16_000.0;
            0.3 * (2.0 * std::f32::consts::PI * (180.0 + seed * 40.0 + 90.0 * (t * 7.0).sin()) * t).sin()
        })
        .collect()
}

fn feed(svc: &VoiceService, audio: &[f32]) {
    for chunk in audio.chunks(320) {
        svc.inject(chunk.to_vec());
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Wartet auf ein Ereignis, das `pred` erfüllt; andere werden übersprungen.
fn wait_for(rx: &Receiver<Event>, secs: u64, pred: impl Fn(&Event) -> bool) -> Option<Event> {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
        if pred(&e) {
            return Some(e);
        }
    }
    None
}

#[test]
fn push_to_talk_records_and_asks_stt() {
    let (svc, rx) = start(config(Mode::PushToTalk));
    // Ohne Taste ist das Mikro zu.
    assert!(wait_for(&rx, 2, |e| matches!(e, Event::Mic(MicState::Off))).is_some());
    svc.send(Ctl::Ptt(true));
    assert!(wait_for(&rx, 2, |e| matches!(e, Event::Mic(MicState::Recording))).is_some());
    feed(&svc, &speech(800, 1.0));
    svc.send(Ctl::Ptt(false));
    // Kein Modell installiert → verständliche Fehlermeldung statt Absturz.
    let e = wait_for(&rx, 5, |e| matches!(e, Event::Error(_))).expect("Fehler erwartet");
    assert!(matches!(e, Event::Error(m) if m.contains("Sprachmodell fehlt")));
    // Danach ist das Mikro wieder zu.
    assert!(wait_for(&rx, 2, |e| matches!(e, Event::Mic(MicState::Off))).is_some());
}

#[test]
fn muted_ignores_push_to_talk() {
    let mut cfg = config(Mode::PushToTalk);
    cfg.muted = true;
    let (svc, rx) = start(cfg);
    assert!(wait_for(&rx, 2, |e| matches!(e, Event::Mic(MicState::Muted))).is_some());
    svc.send(Ctl::Ptt(true));
    feed(&svc, &speech(500, 1.0));
    svc.send(Ctl::Ptt(false));
    assert!(wait_for(&rx, 2, |e| matches!(e, Event::Mic(MicState::Recording) | Event::Error(_))).is_none());
}

#[test]
fn kill_switch_closes_mic_immediately() {
    let (svc, rx) = start(config(Mode::PushToTalk));
    svc.send(Ctl::Ptt(true));
    assert!(wait_for(&rx, 2, |e| matches!(e, Event::Mic(MicState::Recording))).is_some());
    let mut cfg = config(Mode::PushToTalk);
    cfg.enabled = false;
    svc.send(Ctl::Config(cfg));
    assert!(wait_for(&rx, 2, |e| matches!(e, Event::Mic(MicState::Off))).is_some());
    feed(&svc, &speech(500, 1.0));
    svc.send(Ctl::Ptt(false));
    assert!(wait_for(&rx, 2, |e| matches!(e, Event::Error(_) | Event::Transcript { .. })).is_none());
}

#[test]
fn wake_word_training_writes_feature_file_only() {
    let mut cfg = config(Mode::Wake);
    cfg.wakeword_path = tmp("train.rpw");
    let _ = std::fs::remove_file(&cfg.wakeword_path);
    let path = cfg.wakeword_path.clone();
    let (svc, rx) = start(cfg);
    for i in 0..6 {
        svc.send(Ctl::TrainStart);
        feed(&svc, &vec![0.0; 3_200]);
        feed(&svc, &speech(600, i as f32));
        feed(&svc, &vec![0.0; 24_000]); // 1,5 s Stille beendet die Probe
        let e = wait_for(&rx, 5, |e| matches!(e, Event::TrainSample { .. } | Event::Error(_))).expect("Probe");
        assert!(matches!(e, Event::TrainSample { count, .. } if count == i + 1), "{e:?}");
    }
    let done = wait_for(&rx, 5, |e| matches!(e, Event::TrainDone(_))).expect("Training fertig");
    assert!(matches!(done, Event::TrainDone(Ok(()))), "{done:?}");
    assert!(path.is_file());
    // Nach dem Training lauscht das Wake-Word (Mikro offen).
    assert!(wait_for(&rx, 2, |e| matches!(e, Event::Mic(MicState::Wake))).is_some());
    let _ = std::fs::remove_file(path);
}

/// Kunstwort als Wake-Word: zwei "Silben" mit Grundton, Obertönen und Tonhöhenverlauf.
fn word(variation: f32) -> Vec<f32> {
    (0..16 * 700)
        .map(|i| {
            let t = i as f32 / 16_000.0;
            let f0 = if t < 0.35 { 140.0 + 60.0 * t } else { 210.0 - 80.0 * (t - 0.35) } * (1.0 + variation);
            let env = (std::f32::consts::PI * (t / 0.35).fract()).sin().max(0.0);
            env * (1..8).map(|h| (2.0 * std::f32::consts::PI * f0 * h as f32 * t).sin() / h as f32).sum::<f32>() * 0.25
        })
        .collect()
}

fn padded(w: &[f32]) -> Vec<f32> {
    let mut a = vec![0.0; 8_000];
    a.extend_from_slice(w);
    a.extend(vec![0.0; 16_000]);
    a
}

#[test]
fn wake_word_from_speakers_is_ignored() {
    let path = tmp("guard.rpw");
    let samples: Vec<Vec<f32>> = (0..5).map(|i| word(i as f32 * 0.004)).collect();
    panco_voice::wake::train(&samples, 0.5, &path).unwrap();
    let mut cfg = config(Mode::Wake);
    cfg.wakeword_path = path.clone();
    let (svc, rx) = start(cfg);

    // Nur am Mikrofon → Panco wacht auf
    feed(&svc, &padded(&word(0.002)));
    assert!(wait_for(&rx, 5, |e| matches!(e, Event::Wake { .. })).is_some(), "Wake-Word am Mikro erkannt");
    svc.send(Ctl::Cancel);

    // Musik/Spielsound im Systemton stört nicht
    let music: Vec<f32> = (0..16 * 3000).map(|i| 0.2 * (i as f32 * 0.17).sin() * (i as f32 * 0.0009).sin()).collect();
    svc.inject_loopback(music, 16_000);
    feed(&svc, &padded(&word(0.002)));
    let e = wait_for(&rx, 5, |e| matches!(e, Event::Wake { .. } | Event::WakeRejected(_)));
    assert!(matches!(e, Some(Event::Wake { .. })), "Musik im Systemton: {e:?}");
    svc.send(Ctl::Cancel);

    // Dasselbe Wort lief gerade auch über die Lautsprecher → ignoriert
    svc.inject_loopback(padded(&word(0.003)), 16_000);
    feed(&svc, &padded(&word(0.002)));
    let e = wait_for(&rx, 5, |e| matches!(e, Event::Wake { .. } | Event::WakeRejected(_)));
    assert!(matches!(e, Some(Event::WakeRejected(panco_voice::Rejected::Speakers))), "{e:?}");
    assert_eq!(svc.stats().rejected_speakers.load(std::sync::atomic::Ordering::Relaxed), 1);
    let _ = std::fs::remove_file(path);
}
