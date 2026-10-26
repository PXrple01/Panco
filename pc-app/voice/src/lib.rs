//! panco-voice – lokale Sprachsteuerung. Kein Audio verlässt den PC, nichts wird gespeichert.
//!
//! - `capture`   Mikrofon (cpal/WASAPI), nur offen, wenn gebraucht
//! - `resample`  → 16 kHz Mono
//! - `vad`       Sprachaktivität (Energie, adaptiv)
//! - `wake`      Wake-Word "Panco" (rustpotter, selbst eingesprochen)
//! - `loopback`  Systemton-Abgleich (kam "Panco" aus den Lautsprechern?)
//! - `speaker`   optionale Sprecherprüfung (Stimmprofil aus den Wake-Word-Proben)
//! - `stt`       whisper.cpp, Modell auf Abruf; `stt_windows` als Vergleich
//! - `tts`       Windows-Sprachausgabe (Tonhöhe/Tempo einstellbar)
//! - `pipeline`  Zustandsautomat, eigener Thread

pub mod capture;
pub mod loopback;
pub mod model;
mod pipeline;
pub mod resample;
pub mod speaker;
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
    /// Ein Befehl (Push-to-Talk).
    Command,
    /// Ein Befehl nach dem Wake-Word – kann eine Fehlaktivierung sein.
    Wake,
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
    /// "Panco" aus den Lautsprechern (Systemton) ignorieren.
    pub loopback_check: bool,
    /// Nur auf die eigene Stimme reagieren (Stimmprofil nötig).
    pub speaker_check: bool,
    /// Stimmprofil (24 Merkmale, keine Audiodaten); entsteht beim Einsprechen.
    pub profile_path: PathBuf,
}

#[derive(Debug)]
pub enum Ctl {
    Config(VoiceConfig),
    /// Push-to-Talk gedrückt (true) / losgelassen (false).
    Ptt(bool),
    /// Nächste Äußerung ohne Wake-Word aufnehmen (Antwort auf Rückfrage).
    ListenOnce,
    /// Laufende Aufnahme verwerfen (Overlay weggeklickt).
    Cancel,
    /// Systemton einspeisen (nur bei `Source::Injected`: Tests, Messwerkzeug), Rate in Hz.
    LoopbackAudio(Vec<f32>, u32),
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
    /// "Panco" erkannt, aber verworfen (Aktivierungs-Schutz).
    WakeRejected(Rejected),
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

/// Warum eine Aktivierung verworfen wurde.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// "Panco" kam auch im Systemton vor (Lautsprecher, Stream, Video).
    Speakers,
    /// Stimme passt nicht zum Stimmprofil.
    OtherVoice,
}

/// Messwerte für das Diagnosefenster.
#[derive(Default)]
pub struct Stats {
    pub wake_cpu_ns: AtomicU64,
    pub wake_audio_ns: AtomicU64,
    pub whisper_loaded: AtomicBool,
    pub last: Mutex<Option<Transcript>>,
    /// Punktzahl der letzten Wake-Word-Erkennung
    pub last_wake: Mutex<Option<f32>>,
    pub rejected_speakers: AtomicU64,
    pub rejected_voice: AtomicU64,
    /// Systemton-Mitschnitt aktiv?
    pub loopback_active: AtomicBool,
    /// Rechenzeit der Schutzprüfungen (Summe, ns) und Anzahl
    pub guard_ns: AtomicU64,
    pub guard_runs: AtomicU64,
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

    /// Systemton einspeisen (nur bei `Source::Injected`).
    pub fn inject_loopback(&self, samples: Vec<f32>, rate: u32) {
        self.send(Ctl::LoopbackAudio(samples, rate));
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
