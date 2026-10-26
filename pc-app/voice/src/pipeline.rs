//! Sprachpipeline als Zustandsautomat in einem eigenen Thread (rein ereignisgesteuert:
//! blockiert auf Audio- oder Steuernachrichten, kein Polling).
//!
//! ```text
//!   Mikrofon ─▶ Resampler 16 kHz ─▶ VAD ─▶ Wake-Word ──"Panco"──▶ Aufnahme bis Stille ─▶ Whisper ─▶ Text
//!                                           ▲ nur bei Geräusch                              (eigener Thread,
//!   Push-to-Talk ────────────────────────────────────────────────▶ Aufnahme bis Loslassen     Modell auf Abruf)
//! ```
//! Audio wird nur im Speicher gehalten und nach der Erkennung verworfen.

use crate::capture::{self, Mic};
use crate::resample::{Resampler, TARGET_RATE};
use crate::stt::{Transcript, WhisperStt};
use crate::vad::{Vad, FRAME};
use crate::wake::{self, WakeDetector};
use crate::{Ctl, Event, MicState, Mode, Purpose, Stats, SttEngine, VoiceConfig};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Whisper-Hinweis für Antworten auf Rückfragen.
const ANSWER_PROMPT: &str = "Ja. Nein. Ja, bitte. Nein, lass das. Okay.";

/// Woher das Audio kommt.
pub enum Source {
    /// Echtes Mikrofon (cpal).
    Microphone,
    /// Audio wird von außen eingespeist (Tests, Messwerkzeug) – mit dieser Rate.
    Injected { rate: u32 },
}

pub(crate) enum Msg {
    Audio(Vec<f32>),
    Ctl(Ctl),
    SttDone(Purpose, Result<Transcript, String>),
}

struct Rec {
    purpose: Purpose,
    buf: Vec<f32>,
    started: Instant,
    speech_frames: u32,
    silence_frames: u32,
    ptt: bool,
    training: bool,
}

enum State {
    Idle,
    Recording(Rec),
    Transcribing,
}

/// Die letzten 2,5 s Audio (nur im Speicher). Nach "Panco" bekommt Whisper die ganze Äußerung ab
/// Sprechbeginn ("Panco, öffne Deadlock") – das erkennt Whisper deutlich besser als einen
/// Ausschnitt, der mitten im Wort beginnt. Das Wake-Word entfernt danach der Parser.
const RING: usize = 40_000;
/// Wake-Word-Vorlauf beim Einsetzen von Geräusch.
const WAKE_PREROLL: usize = 6_400; // 400 ms
/// Etwas Luft vor dem erkannten Sprechbeginn.
const ONSET_MARGIN: usize = 3_200; // 200 ms

struct Pipeline {
    cfg: VoiceConfig,
    source: Source,
    tx: Sender<Msg>,
    events: Box<dyn Fn(Event) + Send>,
    stats: Arc<Stats>,
    stt_tx: Sender<SttJob>,

    mic: Option<Mic>,
    /// Bei `Source::Injected`: gilt das "Mikrofon" als offen?
    injected_open: bool,
    mic_failed: bool,
    mic_state: MicState,
    /// Nach einer Konfigänderung den Zustand einmal neu melden.
    force_mic_event: bool,
    resampler: Resampler,
    vad: Vad,
    wake: Option<WakeDetector>,
    state: State,
    pending16: Vec<f32>,
    ring: VecDeque<f32>,
    wake_fed: bool,
    silent_frames: u32,
    /// Samples seit Beginn der aktuellen Äußerung (VAD), 0 = gerade Stille.
    utterance_len: usize,
    last_level: Instant,
    train: Option<Vec<Vec<f32>>>,
}

pub(crate) enum SttJob {
    Whisper {
        pcm: Vec<f32>,
        prompt: String,
        purpose: Purpose,
        model: std::path::PathBuf,
    },
    #[cfg_attr(not(windows), allow(dead_code))]
    Windows {
        phrases: Vec<String>,
        purpose: Purpose,
    },
}

pub(crate) fn spawn(
    cfg: VoiceConfig,
    source: Source,
    events: Box<dyn Fn(Event) + Send>,
    stats: Arc<Stats>,
) -> (Sender<Msg>, std::thread::JoinHandle<()>) {
    let (tx, rx) = channel::<Msg>();
    let stt_tx = spawn_stt(tx.clone(), stats.clone(), cfg.keep_model_secs);
    let rate = match source {
        Source::Injected { rate } => rate,
        Source::Microphone => TARGET_RATE,
    };
    let mut p = Pipeline {
        cfg,
        source,
        tx: tx.clone(),
        events,
        stats,
        stt_tx,
        mic: None,
        injected_open: false,
        mic_failed: false,
        mic_state: MicState::Off,
        force_mic_event: true,
        resampler: Resampler::new(rate),
        vad: Vad::default(),
        wake: None,
        state: State::Idle,
        pending16: Vec::with_capacity(FRAME * 4),
        ring: VecDeque::with_capacity(RING),
        wake_fed: false,
        silent_frames: 0,
        utterance_len: 0,
        last_level: Instant::now(),
        train: None,
    };
    let handle = std::thread::Builder::new()
        .name("panco-voice".into())
        .spawn(move || {
            p.reload_wake();
            p.sync_mic();
            p.run(rx);
        })
        .expect("Sprach-Thread startet");
    (tx, handle)
}

impl Pipeline {
    fn run(&mut self, rx: Receiver<Msg>) {
        for msg in rx {
            match msg {
                Msg::Audio(chunk) => self.on_audio(&chunk),
                Msg::Ctl(Ctl::Shutdown) => break,
                Msg::Ctl(ctl) => self.on_ctl(ctl),
                Msg::SttDone(purpose, result) => self.on_stt(purpose, result),
            }
        }
        self.mic = None;
    }

    fn emit(&self, e: Event) {
        (self.events)(e);
    }

    // ---------- Steuerung ----------

    fn on_ctl(&mut self, ctl: Ctl) {
        match ctl {
            Ctl::Config(cfg) => {
                let wake_changed = cfg.wakeword_path != self.cfg.wakeword_path || cfg.threshold != self.cfg.threshold;
                let disabled = !cfg.enabled || cfg.muted;
                self.cfg = cfg;
                self.mic_failed = false;
                self.force_mic_event = true;
                if wake_changed || self.wake.is_none() {
                    self.reload_wake();
                }
                if disabled {
                    // Not-Aus/Stumm: laufende Aufnahme sofort verwerfen.
                    self.state = State::Idle;
                    self.train = None;
                }
            }
            Ctl::Ptt(true) => {
                if self.cfg.enabled && !self.cfg.muted && self.cfg.mode != Mode::Off {
                    self.start_listening(Purpose::Command, true);
                }
            }
            Ctl::Ptt(false) => {
                if let State::Recording(rec) = &self.state {
                    if rec.ptt {
                        self.finish_recording();
                    }
                }
            }
            Ctl::ListenOnce if self.cfg.enabled && !self.cfg.muted && self.cfg.mode != Mode::Off => {
                match &mut self.state {
                    State::Idle => self.start_listening(Purpose::Answer, false),
                    // Hört schon (Dazwischenreden), aber noch nichts gesagt: Wartezeit ab jetzt neu
                    // (z. B. wenn Panco gerade fertig gesprochen hat).
                    State::Recording(rec) if rec.purpose == Purpose::Answer && rec.speech_frames < 5 => {
                        rec.started = Instant::now();
                    }
                    _ => {}
                }
            }
            Ctl::ListenOnce => {}
            Ctl::Cancel => {
                if matches!(self.state, State::Recording(ref rec) if !rec.training) {
                    self.state = State::Idle;
                    self.emit(Event::NoSpeech);
                }
            }
            Ctl::TrainStart => {
                if self.cfg.enabled && !self.cfg.muted {
                    self.train.get_or_insert_with(Vec::new);
                    self.state = State::Recording(Rec::new(Purpose::Command, false, true));
                }
            }
            Ctl::TrainCancel => {
                self.train = None;
                self.state = State::Idle;
            }
            Ctl::Shutdown => {}
        }
        self.sync_mic();
    }

    fn start_listening(&mut self, purpose: Purpose, ptt: bool) {
        if self.cfg.engine == SttEngine::Windows {
            self.state = State::Transcribing;
            let _ = self.stt_tx.send(SttJob::Windows { phrases: self.cfg.phrases.clone(), purpose });
        } else {
            self.state = State::Recording(Rec::new(purpose, ptt, false));
        }
    }

    fn reload_wake(&mut self) {
        self.wake = None;
        if self.cfg.wakeword_path.is_file() {
            match WakeDetector::load(&self.cfg.wakeword_path, self.cfg.threshold) {
                Ok(w) => self.wake = Some(w),
                Err(e) => self.emit(Event::Error(e)),
            }
        }
    }

    /// Mikrofon nur öffnen, wenn es gebraucht wird – sonst ist es komplett zu.
    fn sync_mic(&mut self) {
        let active = !matches!(self.state, State::Idle) || self.train.is_some();
        let listening = self.cfg.mode == Mode::Wake && self.wake.is_some();
        let want = self.cfg.enabled && !self.cfg.muted && (listening || active);

        let is_open = self.mic.is_some() || self.injected_open;
        if want && !is_open && !self.mic_failed {
            match self.source {
                Source::Microphone => {
                    let tx = self.tx.clone();
                    match capture::open(move |chunk| {
                        let _ = tx.send(Msg::Audio(chunk));
                    }) {
                        Ok(mic) => {
                            self.resampler = Resampler::new(mic.rate);
                            self.mic = Some(mic);
                        }
                        Err(e) => {
                            self.mic_failed = true;
                            self.emit(Event::Error(e));
                        }
                    }
                }
                Source::Injected { .. } => self.injected_open = true,
            }
            self.vad = Vad::default();
            self.ring.clear();
            self.pending16.clear();
            self.wake_fed = false;
        } else if !want && is_open {
            self.mic = None; // Stream fallen lassen = Mikrofon zu
            self.injected_open = false;
        }
        let is_open = self.mic.is_some() || self.injected_open;

        let state = if self.cfg.muted {
            MicState::Muted
        } else if !want || !is_open {
            MicState::Off
        } else if matches!(self.state, State::Recording(_)) {
            MicState::Recording
        } else {
            MicState::Wake
        };
        if state != self.mic_state || std::mem::take(&mut self.force_mic_event) {
            self.mic_state = state;
            self.emit(Event::Mic(state));
        }
    }

    // ---------- Audio ----------

    fn on_audio(&mut self, chunk: &[f32]) {
        if self.mic.is_none() && !self.injected_open {
            return; // Nachzügler nach dem Schließen
        }
        let t0 = Instant::now();
        let idle = matches!(self.state, State::Idle);
        self.resampler.process(chunk, &mut self.pending16);
        let mut start = 0;
        while self.pending16.len() - start >= FRAME {
            let frame: Vec<f32> = self.pending16[start..start + FRAME].to_vec();
            start += FRAME;
            self.on_frame(&frame);
        }
        self.pending16.drain(..start);

        if idle {
            let audio_ns = chunk.len() as u64 * 1_000_000_000 / self.input_rate() as u64;
            self.stats.wake_audio_ns.fetch_add(audio_ns, Ordering::Relaxed);
            self.stats.wake_cpu_ns.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
    }

    fn input_rate(&self) -> u32 {
        match (&self.source, &self.mic) {
            (Source::Injected { rate }, _) => *rate,
            (_, Some(m)) => m.rate,
            _ => TARGET_RATE,
        }
    }

    fn on_frame(&mut self, frame: &[f32]) {
        let f = self.vad.frame(frame);
        if self.ring.len() + frame.len() > RING {
            let excess = self.ring.len() + frame.len() - RING;
            self.ring.drain(..excess);
        }
        self.ring.extend(frame.iter().copied());
        self.utterance_len = if f.speech { self.utterance_len + frame.len() } else { 0 };

        match &mut self.state {
            State::Idle => {
                let Some(wake) = self.wake.as_mut() else { return };
                if self.cfg.mode != Mode::Wake {
                    return;
                }
                // Wake-Word nur bei Geräusch rechnen; beim Einsetzen den Vorlauf nachreichen.
                let mut hit = None;
                if f.speech {
                    self.silent_frames = 0;
                    if !self.wake_fed {
                        wake.reset();
                        let pre: Vec<f32> = self.ring.iter().rev().take(WAKE_PREROLL).rev().copied().collect();
                        hit = wake.process(&pre);
                        self.wake_fed = true;
                    } else {
                        hit = wake.process(frame);
                    }
                } else if self.wake_fed {
                    hit = wake.process(frame);
                    self.silent_frames += 1;
                    if self.silent_frames > 50 {
                        self.wake_fed = false; // 1 s Ruhe: pausieren
                    }
                }
                if let Some(score) = hit {
                    self.wake_fed = false;
                    self.emit(Event::Wake { score });
                    self.start_listening(Purpose::Command, false);
                    if let State::Recording(rec) = &mut self.state {
                        let take = (self.utterance_len + ONSET_MARGIN).min(self.ring.len());
                        rec.buf.extend(self.ring.iter().rev().take(take).rev());
                    }
                    self.sync_mic();
                }
            }
            State::Recording(rec) => {
                rec.buf.extend_from_slice(frame);
                if f.speech {
                    rec.speech_frames += 1;
                    rec.silence_frames = 0;
                } else {
                    rec.silence_frames += 1;
                }
                if self.last_level.elapsed() >= Duration::from_millis(100) {
                    self.last_level = Instant::now();
                    (self.events)(Event::Level(f.level));
                }
                if rec.ptt {
                    if rec.started.elapsed() > Duration::from_secs(15) {
                        self.finish_recording();
                    }
                    return;
                }
                let elapsed = rec.started.elapsed();
                let (max_wait, end_silence, max_len) = if rec.training {
                    (Duration::from_secs(4), 20, Duration::from_millis(2500))
                } else if rec.purpose == Purpose::Answer {
                    (Duration::from_secs(6), 30, Duration::from_secs(5))
                } else {
                    (Duration::from_secs(4), 35, Duration::from_secs(10))
                };
                let spoke = rec.speech_frames >= 5;
                if (spoke && rec.silence_frames >= end_silence) || (spoke && elapsed > max_len) {
                    self.finish_recording();
                } else if !spoke && elapsed > max_wait {
                    self.state = State::Idle;
                    self.emit(Event::NoSpeech);
                    self.sync_mic();
                }
            }
            State::Transcribing => {}
        }
    }

    fn finish_recording(&mut self) {
        let State::Recording(rec) = std::mem::replace(&mut self.state, State::Idle) else { return };
        if rec.training {
            self.finish_training_sample(rec.buf);
        } else if rec.buf.len() < TARGET_RATE as usize * 3 / 10 {
            self.emit(Event::NoSpeech);
        } else {
            self.state = State::Transcribing;
            // Antwort auf eine Rückfrage: kurzer Ja/Nein-Hinweis – mit dem langen Befehls-Hinweis
            // wird ein kurzes "Ja" gern zu "Jan." (gemessen mit stt_eval).
            let prompt = match rec.purpose {
                Purpose::Answer => ANSWER_PROMPT.to_string(),
                Purpose::Command => self.cfg.prompt.clone(),
            };
            let _ = self.stt_tx.send(SttJob::Whisper {
                pcm: rec.buf,
                prompt,
                purpose: rec.purpose,
                model: self.cfg.model_path.clone(),
            });
        }
        self.sync_mic();
    }

    fn finish_training_sample(&mut self, buf: Vec<f32>) {
        let sample = wake::trim_silence(&buf);
        let Some(samples) = self.train.as_mut() else { return };
        if sample.len() < 3_200 {
            self.emit(Event::Error("Zu leise – bitte noch einmal „Panco“ sagen.".into()));
            return;
        }
        samples.push(sample);
        let count = samples.len();
        self.emit(Event::TrainSample { count, needed: wake::SAMPLES_NEEDED });
        if count >= wake::SAMPLES_NEEDED {
            let samples = self.train.take().unwrap_or_default();
            let result = wake::train(&samples, self.cfg.threshold, &self.cfg.wakeword_path);
            drop(samples); // Aufnahmen verwerfen – nur die Merkmalsdatei bleibt
            if result.is_ok() {
                self.reload_wake();
            }
            self.emit(Event::TrainDone(result));
        }
    }

    fn on_stt(&mut self, purpose: Purpose, result: Result<Transcript, String>) {
        self.state = State::Idle;
        match result {
            Ok(t) if t.text.trim().is_empty() => self.emit(Event::NoSpeech),
            Ok(t) => {
                *self.stats.last.lock().unwrap() = Some(t.clone());
                self.emit(Event::Transcript { purpose, transcript: t });
            }
            Err(e) => self.emit(Event::Error(e)),
        }
        self.sync_mic();
    }
}

impl Rec {
    fn new(purpose: Purpose, ptt: bool, training: bool) -> Self {
        Rec {
            purpose,
            buf: Vec::with_capacity(TARGET_RATE as usize * 4),
            started: Instant::now(),
            speech_frames: 0,
            silence_frames: 0,
            ptt,
            training,
        }
    }
}

/// Erkennungs-Thread: hält Whisper nur so lange geladen wie nötig.
fn spawn_stt(back: Sender<Msg>, stats: Arc<Stats>, keep_secs: u64) -> Sender<SttJob> {
    let (tx, rx) = channel::<SttJob>();
    std::thread::Builder::new()
        .name("panco-stt".into())
        .spawn(move || {
            let mut whisper = WhisperStt::new(Default::default());
            let keep = Duration::from_secs(keep_secs.max(1));
            loop {
                // Ist ein Modell geladen, nach `keep` ohne Auftrag freigeben; sonst einfach warten.
                let job = if whisper.loaded() {
                    match rx.recv_timeout(keep) {
                        Ok(j) => j,
                        Err(RecvTimeoutError::Timeout) => {
                            whisper.unload();
                            stats.whisper_loaded.store(false, Ordering::Relaxed);
                            release_heap();
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                } else {
                    match rx.recv() {
                        Ok(j) => j,
                        Err(_) => break,
                    }
                };
                let (purpose, result) = match job {
                    SttJob::Whisper { pcm, prompt, purpose, model } => {
                        whisper.set_model(model);
                        let r = whisper.transcribe(&pcm, &prompt);
                        stats.whisper_loaded.store(whisper.loaded(), Ordering::Relaxed);
                        (purpose, r)
                    }
                    SttJob::Windows { phrases, purpose } => {
                        #[cfg(windows)]
                        let r = crate::stt_windows::recognize_once(&phrases);
                        #[cfg(not(windows))]
                        let r = {
                            let _ = phrases;
                            Err("Windows-Spracherkennung gibt es nur unter Windows".to_string())
                        };
                        (purpose, r)
                    }
                };
                if back.send(Msg::SttDone(purpose, result)).is_err() {
                    break;
                }
            }
        })
        .expect("STT-Thread startet");
    tx
}

/// Freigegebenen Speicher wirklich an das System zurückgeben. Unter Windows passiert das für große
/// Blöcke von selbst; glibc (Linux) behält ihn sonst im Prozess.
fn release_heap() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        // SAFETY: malloc_trim ist threadsicher und hat keine Vorbedingungen.
        unsafe {
            malloc_trim(0);
        }
    }
}
