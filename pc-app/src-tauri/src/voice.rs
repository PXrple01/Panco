//! Brücke zwischen Sprachpipeline (panco-voice) und App: Konfiguration, Ereignisse,
//! Sprachausgabe, Rückfragen per Stimme.

use crate::{overlay, tray};
use panco_core::config::{models_dir, wakeword_path, SttChoice, VoiceMode};
use panco_core::protocol::{AvatarState, MicState, ServerMessage};
use panco_core::{Engine, Source};
use panco_voice::tts::Speaker;
use panco_voice::{model, Ctl, Event, Mode, Purpose, SttEngine, VoiceConfig, VoiceService};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

pub struct Voice {
    pub svc: VoiceService,
    speaker: Speaker,
    pub muted: AtomicBool,
    pub mic: Mutex<MicState>,
    pub avatar: Mutex<AvatarState>,
    counter: AtomicU64,
}

pub fn get(app: &AppHandle) -> tauri::State<'_, Voice> {
    app.state::<Voice>()
}

/// Wie `get`, aber ohne Panik, falls die Pipeline schon meldet, bevor der Zustand registriert ist.
fn try_get(app: &AppHandle) -> Option<tauri::State<'_, Voice>> {
    app.try_state::<Voice>()
}

pub fn start(app: &AppHandle) {
    let handle = app.clone();
    let cfg = build_config(app, false);
    let svc = VoiceService::start(cfg, panco_voice::Source::Microphone, move |e| on_event(&handle, e));
    app.manage(Voice {
        svc,
        speaker: Speaker::new(),
        muted: AtomicBool::new(false),
        mic: Mutex::new(MicState::Off),
        avatar: Mutex::new(AvatarState::Idle),
        counter: AtomicU64::new(0),
    });
    // Erst jetzt ist der Zustand registriert: Konfig erneut senden, damit die Pipeline ihren
    // Mikrofonzustand meldet.
    apply(app);
}

/// Aktuelle Einstellungen (Konfig, Not-Aus, Stumm) an die Pipeline geben.
pub fn apply(app: &AppHandle) {
    let Some(v) = try_get(app) else { return };
    let muted = v.muted.load(Ordering::Relaxed);
    v.svc.send(Ctl::Config(build_config(app, muted)));
    tray::refresh(app);
}

fn build_config(app: &AppHandle, muted: bool) -> VoiceConfig {
    let engine = crate::engine(app);
    let v = engine.config().voice;
    VoiceConfig {
        mode: match v.mode {
            VoiceMode::Wake => Mode::Wake,
            VoiceMode::PushToTalk => Mode::PushToTalk,
            VoiceMode::Off => Mode::Off,
        },
        enabled: engine.enabled(),
        muted,
        engine: match v.engine {
            SttChoice::Whisper => SttEngine::Whisper,
            SttChoice::Windows => SttEngine::Windows,
        },
        wakeword_path: wakeword_path(),
        threshold: v.wake_threshold.clamp(0.2, 0.9),
        model_path: model::path(&models_dir(), &v.model),
        keep_model_secs: v.keep_model_secs.clamp(5, 600),
        prompt: whisper_prompt(&engine),
        phrases: command_phrases(&engine),
    }
}

/// Vokabular-Hinweis für Whisper: echte Spiel- und Programmnamen verbessern die Erkennung stark.
fn whisper_prompt(engine: &Engine) -> String {
    let mut names: Vec<String> = engine.games().into_iter().map(|g| g.name).collect();
    names.extend(engine.config().apps.iter().filter_map(|a| a.names.first().cloned()));
    let mut list = String::new();
    for n in names {
        if list.len() + n.len() > 500 {
            break;
        }
        list.push_str(&n);
        list.push_str(", ");
    }
    format!(
        "Panco, öffne Deadlock. Starte Discord. Lautstärke auf 30 Prozent. Nächstes Lied. Such Build Guides. \
         Fahr den PC herunter. Ja. Nein. {}",
        list.trim_end_matches(", ")
    )
}

/// Phrasenliste für die Windows-Erkennung (die kann nur Bekanntes erkennen).
fn command_phrases(engine: &Engine) -> Vec<String> {
    let mut targets: Vec<String> = engine.games().into_iter().map(|g| g.name).collect();
    targets.extend(engine.config().apps.iter().flat_map(|a| a.names.clone()));
    let mut p: Vec<String> = Vec::new();
    for t in &targets {
        for verb in ["öffne", "starte", "spiel", "schließe", "wechsel zu"] {
            p.push(format!("{verb} {t}"));
        }
    }
    for n in (0..=100).step_by(10) {
        p.push(format!("lautstärke {n}"));
        p.push(format!("lautstärke auf {n} prozent"));
    }
    let fixed = [
        "lauter",
        "leiser",
        "ton aus",
        "ton an",
        "pause",
        "weiter",
        "nächstes lied",
        "vorheriges lied",
        "pc sperren",
        "screenshot",
        "fahr den pc herunter",
        "starte den pc neu",
        "ruhemodus",
        "ja",
        "nein",
    ];
    p.extend(fixed.iter().map(|s| s.to_string()));
    p
}

fn set_state(app: &AppHandle, avatar: Option<AvatarState>, mic: Option<MicState>, level: Option<f32>) {
    let Some(v) = try_get(app) else { return };
    if let Some(a) = avatar {
        *v.avatar.lock().unwrap() = a;
    }
    if let Some(m) = mic {
        *v.mic.lock().unwrap() = m;
    }
    let msg = ServerMessage::State {
        avatar: *v.avatar.lock().unwrap(),
        mic: *v.mic.lock().unwrap(),
        enabled: crate::engine(app).enabled(),
        level,
    };
    overlay::emit(app, &msg);
}

#[derive(Clone, Serialize)]
struct Train {
    count: usize,
    needed: usize,
    done: bool,
    error: Option<String>,
}

fn on_event(app: &AppHandle, event: Event) {
    if try_get(app).is_none() {
        return;
    }
    let show_overlay = crate::engine(app).config().voice.show_overlay;
    match event {
        Event::Mic(m) => {
            let m = match m {
                panco_voice::MicState::Off => MicState::Off,
                panco_voice::MicState::Wake => MicState::Wake,
                panco_voice::MicState::Recording => MicState::Recording,
                panco_voice::MicState::Muted => MicState::Muted,
            };
            set_state(app, None, Some(m), None);
            tray::refresh(app);
        }
        Event::Wake { .. } => {
            // Overlay ohne Fokus: ein laufendes Spiel bleibt im Vordergrund.
            if show_overlay {
                overlay::show(app, false);
            }
            set_state(app, Some(AvatarState::Listening), None, None);
        }
        Event::Level(level) => set_state(app, None, None, Some(level)),
        Event::NoSpeech => set_state(app, Some(AvatarState::Idle), None, None),
        Event::Transcript { purpose, transcript } => {
            set_state(app, Some(AvatarState::Thinking), None, None);
            let _ = app.emit_to(overlay::LABEL, "panco://heard", &transcript.text);
            let app = app.clone();
            std::thread::spawn(move || handle_transcript(&app, purpose, &transcript.text));
        }
        Event::TrainSample { count, needed } => {
            let _ = app.emit("panco://train", Train { count, needed, done: false, error: None });
        }
        Event::TrainDone(result) => {
            let error = result.err();
            let _ = app.emit("panco://train", Train { count: 0, needed: 0, done: true, error });
            tray::refresh(app);
        }
        Event::Error(e) => {
            eprintln!("[panco] Sprache: {e}");
            let _ = app.emit("panco://voice-error", &e);
            set_state(app, Some(AvatarState::Idle), None, None);
        }
    }
}

/// Erkannten Text wie einen getippten Befehl verarbeiten – mit Quelle "voice".
fn handle_transcript(app: &AppHandle, _purpose: Purpose, text: &str) {
    let v = get(app);
    let id = format!("v{}", v.counter.fetch_add(1, Ordering::Relaxed) + 1);
    let msg = crate::engine(app).command(&id, text, Source::Voice);
    set_state(app, Some(AvatarState::Idle), None, None);
    overlay::emit(app, &msg);

    let speak = crate::engine(app).config().voice.speak_replies;
    // Bei einer Rückfrage, die per Stimme beantwortet werden darf: nach dem Vorlesen zuhören.
    let listen_after = matches!(msg, ServerMessage::ConfirmRequest { pc_only: false, .. });
    let follow_up: Option<panco_voice::tts::Done> = listen_after.then(|| {
        let app = app.clone();
        Box::new(move || get(&app).svc.send(Ctl::ListenOnce)) as panco_voice::tts::Done
    });
    if speak {
        v.speaker.say(msg.text(), follow_up);
    } else if let Some(f) = follow_up {
        f();
    }
}

pub fn set_muted(app: &AppHandle, muted: bool) {
    get(app).muted.store(muted, Ordering::Relaxed);
    apply(app);
}

pub fn toggle_muted(app: &AppHandle) {
    let muted = !get(app).muted.load(Ordering::Relaxed);
    set_muted(app, muted);
}

pub fn wakeword_ready() -> bool {
    wakeword_path().is_file()
}
