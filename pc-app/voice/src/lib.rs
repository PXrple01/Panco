//! panco-voice – lokale Sprachsteuerung. Kein Audio verlässt den PC, nichts wird gespeichert.
//!
//! - `capture`   Mikrofon (cpal/WASAPI), nur offen, wenn gebraucht
//! - `resample`  → 16 kHz Mono
//! - `vad`       Sprachaktivität (Energie, adaptiv)
//! - `wake`      Wake-Word "Panco" (rustpotter, selbst eingesprochen)
//! - `stt`       whisper.cpp, Modell auf Abruf; `stt_windows` als Vergleich
//! - `tts`       Windows-Sprachausgabe (SAPI)
//! - `pipeline`  Zustandsautomat, eigener Thread

pub mod capture;
pub mod model;
mod pipeline;
pub mod resample;
pub mod stt;
#[cfg(windows)]
mod stt_windows;
pub mod tts;
pub mod vad;
pub mod wake;
pub mod wav;

pub use pipeline::Source;
pub use stt::Transcript;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Offenes Mikro, wartet auf "Panco".
    Wake,
    /// Mikro nur, solange die Taste gehalten wird.
    PushToTalk,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttEngine {
    Whisper,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicState {
    Off,
    /// Mikro offen, nur das Wake-Word lauscht.
    Wake,
    /// Befehl wird aufgenommen.
    Recording,
    Muted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Ein Befehl.
    Command,
    /// Antwort auf eine Rückfrage (ja/nein).
    Answer,
}

#[derive(Debug, Clone)]
pub struct VoiceConfig {
    pub mode: Mode,
    /// false = Not-Aus.
    pub enabled: bool,
    pub muted: bool,
    pub engine: SttEngine,
    pub wakeword_path: PathBuf,
    pub threshold: f32,
    pub model_path: PathBuf,
    pub keep_model_secs: u64,
    /// Vokabular-Hinweis für Whisper (Spielnamen, Befehlswörter).
    pub prompt: String,
    /// Phrasenliste für die Windows-Erkennung.
    pub phrases: Vec<String>,
}

#[derive(Debug)]
pub enum Ctl {
    Config(VoiceConfig),
    /// Push-to-Talk gedrückt (true) / losgelassen (false).
    Ptt(bool),
    /// Nächste Äußerung ohne Wake-Word aufnehmen (Antwort auf Rückfrage).
    ListenOnce,
    /// Eine Wake-Word-Probe aufnehmen.
    TrainStart,
    TrainCancel,
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum Event {
    Mic(MicState),
    Wake {
        score: f32,
    },
    /// Pegel 0..1 während der Aufnahme (~10×/s).
    Level(f32),
    /// Nichts (Verständliches) gesagt.
    NoSpeech,
    Transcript {
        purpose: Purpose,
        transcript: Transcript,
    },
    TrainSample {
        count: usize,
        needed: usize,
    },
    TrainDone(Result<(), String>),
    Error(String),
}

/// Messwerte für das Diagnosefenster.
#[derive(Default)]
pub struct Stats {
    pub wake_cpu_ns: AtomicU64,
    pub wake_audio_ns: AtomicU64,
    pub whisper_loaded: AtomicBool,
    pub last: Mutex<Option<Transcript>>,
}

impl Stats {
    /// Rechenzeit des Wake-Word-Lauschens im Verhältnis zur Audiozeit (= Anteil eines Kerns).
    pub fn wake_cpu_percent(&self) -> f64 {
        let audio = self.wake_audio_ns.load(Ordering::Relaxed);
        if audio == 0 {
            return 0.0;
        }
        self.wake_cpu_ns.load(Ordering::Relaxed) as f64 / audio as f64 * 100.0
    }
}

pub struct VoiceService {
    tx: Sender<pipeline::Msg>,
    stats: Arc<Stats>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl VoiceService {
    pub fn start(cfg: VoiceConfig, source: Source, events: impl Fn(Event) + Send + 'static) -> Self {
        let stats = Arc::new(Stats::default());
        let (tx, thread) = pipeline::spawn(cfg, source, Box::new(events), stats.clone());
        Self { tx, stats, thread: Some(thread) }
    }

    pub fn send(&self, ctl: Ctl) {
        let _ = self.tx.send(pipeline::Msg::Ctl(ctl));
    }

    /// Audio einspeisen (nur bei `Source::Injected`: Tests, Messwerkzeug).
    pub fn inject(&self, samples: Vec<f32>) {
        let _ = self.tx.send(pipeline::Msg::Audio(samples));
    }

    pub fn stats(&self) -> &Arc<Stats> {
        &self.stats
    }
}

impl Drop for VoiceService {
    fn drop(&mut self) {
        self.send(Ctl::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
