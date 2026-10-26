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
        prompt: String::new(),
        phrases: vec![],
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
