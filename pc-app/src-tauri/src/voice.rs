//! Brücke zwischen Sprachpipeline (panco-voice) und App: Konfiguration, Ereignisse,
//! Sprachausgabe, Rückfragen per Stimme.

use crate::{overlay, tray};
use panco_core::config::{models_dir, wakeword_path, SttChoice, VoiceMode};
use panco_core::protocol::{AvatarState, MicState, ServerMessage};
use panco_core::{Engine, Source};
use panco_voice::tts::{Speaker, TtsSettings};
use panco_voice::{model, Ctl, Event, Mode, Purpose, SttEngine, VoiceConfig, VoiceService};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

pub struct Voice {
    pub svc: VoiceService,
    speaker: Speaker,
    /// Gerade vorgelesene Rückfrage – um die eigene Stimme (Lautsprecher-Echo) zu erkennen.
    asked: Mutex<Option<String>>,
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
        asked: Mutex::new(None),
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
    let s = crate::engine(app).config().voice;
    v.speaker.configure(TtsSettings { voice: s.tts_voice, pitch: s.tts_pitch, rate: s.tts_rate });
    tray::refresh(app);
}

/// Probe der eingestellten Stimme (Einstellungen).
pub fn preview(app: &AppHandle) {
    get(app).speaker.say("Hallo! So klinge ich. Sag einfach meinen Namen, wenn du etwas brauchst.", None);
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
        keep_model_secs: v.keep_model_secs.clamp(5, 3600),
        prompt: whisper_prompt(&engine),
        phrases: command_phrases(&engine),
    }
}

/// Vokabular-Hinweis für Whisper: echte Spiel- und Programmnamen verbessern die Erkennung stark.
fn whisper_prompt(engine: &Engine) -> String {
    let mut names: Vec<String> = engine.games().into_iter().map(|g| g.name).collect();
    names.extend(engine.config().apps.iter().filter_map(|a| a.names.first().cloned()));
    // Titel normaler Notizen (bleibt lokal); sensible Titel nie.
    names.extend(panco_core::notes::list(&engine.places().notes).into_iter().filter(|n| !n.sensitive).map(|n| n.title));
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
         Finde meine letzte Rechnung. Neue Notiz Einkauf: Milch. Lies die Notiz vor. Lösche die Datei. \
         Mach das rückgängig. Wie spät ist es? Was kannst du? Fahr den PC herunter. Ja. Nein. {}",
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
        "finde meine letzte rechnung",
        "welche notizen habe ich",
        "mach das rückgängig",
        "wie spät ist es",
        "wie viel uhr ist es",
        "welcher tag ist heute",
        "was kannst du",
        "rückgängig",
        "ja",
        "nein",
    ];
    p.extend(fixed.iter().map(|s| s.to_string()));
    for n in panco_core::notes::list(&engine.places().notes).into_iter().filter(|n| !n.sensitive).take(30) {
        p.push(format!("lies die notiz {} vor", n.title.to_lowercase()));
    }
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
            // "Panco, …" unterbricht, was Panco gerade sagt.
            if let Some(v) = try_get(app) {
                v.speaker.stop();
            }
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
            overlay::heard(app, &without_wake_word(&transcript.text));
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
fn handle_transcript(app: &AppHandle, purpose: Purpose, text: &str) {
    let v = get(app);
    let mut text = text.to_string();
    if purpose == Purpose::Answer {
        let asked = v.asked.lock().unwrap().clone();
        match panco_core::parser::answer_text(&text, asked.as_deref()) {
            Some(t) => text = t,
            None => {
                // Nur die eigene Stimme aus dem Lautsprecher gehört: weiter auf die Antwort warten.
                set_state(app, Some(AvatarState::Idle), None, None);
                if !v.speaker.is_speaking() {
                    v.svc.send(Ctl::ListenOnce);
                }
                return;
            }
        }
        // Eine Antwort ist da: Panco hört auf zu sprechen.
        v.speaker.stop();
    }
    *v.asked.lock().unwrap() = None;

    let id = format!("v{}", v.counter.fetch_add(1, Ordering::Relaxed) + 1);
    let msg = crate::engine(app).command(&id, &text, Source::Voice);
    set_state(app, Some(AvatarState::Idle), None, None);
    overlay::emit(app, &msg);

    let voice = crate::engine(app).config().voice;
    // Bei einer Rückfrage, die per Stimme beantwortet werden darf: zuhören – mit "barge_in"
    // schon während Panco spricht, sonst erst danach.
    let question = matches!(msg, ServerMessage::ConfirmRequest { pc_only: false, .. });
    if question {
        *v.asked.lock().unwrap() = Some(msg.text().to_string());
        if voice.barge_in || !voice.speak_replies {
            v.svc.send(Ctl::ListenOnce);
        }
    }
    let follow_up: Option<panco_voice::tts::Done> = question.then(|| {
        let app = app.clone();
        Box::new(move || get(&app).svc.send(Ctl::ListenOnce)) as panco_voice::tts::Done
    });
    if voice.speak_replies {
        v.speaker.say(msg.text(), follow_up);
    }
}

/// "Paanco, neue Notiz …" → "Neue Notiz …" (zur Anzeige; das Wake-Word hat Panco ja schon erkannt).
fn without_wake_word(text: &str) -> String {
    let t = text.trim();
    match t.split_once(char::is_whitespace) {
        Some((first, rest)) if panco_core::normalize::is_wake_word(first.trim_end_matches([',', '.', '!'])) => {
            let rest = rest.trim_start_matches([',', ' ']);
            let mut c = rest.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
        _ => t.to_string(),
    }
}

/// Weggeklickt: Zuhören und Vorlesen sofort beenden.
pub fn cancel(app: &AppHandle) {
    let Some(v) = try_get(app) else { return };
    *v.asked.lock().unwrap() = None;
    v.speaker.stop();
    v.svc.send(Ctl::Cancel);
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
