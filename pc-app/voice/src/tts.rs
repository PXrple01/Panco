//! Sprachausgabe, lokal: natürliche Stimme (Piper, siehe `natural`) oder die Windows-Stimmen als
//! Rückfall. Tonhöhe und Tempo sind einstellbar – Panco spricht etwas höher, passend zum Avatar.
//! Läuft in einem eigenen Thread; ein neuer Satz und `stop()` brechen den laufenden sofort ab
//! (Dazwischenreden bei Rückfragen). Die natürliche Stimme spricht Satz für Satz: Der erste Satz
//! spielt schon, während der nächste entsteht.

use crate::natural::{self, Piper};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub type Done = Box<dyn FnOnce() + Send>;

#[derive(Debug, Clone, PartialEq)]
pub struct TtsSettings {
    /// Anzeigename der Stimme ("Microsoft Katja"); leer = automatisch eine deutsche Frauenstimme.
    pub voice: String,
    /// 0.5 … 2.0, 1.0 = normal
    pub pitch: f32,
    /// 0.5 … 2.0, 1.0 = normal
    pub rate: f32,
    /// Natürliche Stimme (piper + Stimmdatei); `None` = Windows-Stimme.
    pub natural: Option<Natural>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Natural {
    pub exe: PathBuf,
    pub model: PathBuf,
    /// 0.7 … 1.6 (höher = heller)
    pub pitch: f32,
    pub rate: f32,
    /// So lange bleibt die Stimme nach dem letzten Satz geladen.
    pub keep_secs: u64,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self { voice: String::new(), pitch: 1.0, rate: 1.0, natural: None }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VoiceInfo {
    pub name: String,
    pub language: String,
    pub female: bool,
}

enum Cmd {
    /// Text, Rückmeldung, Generation (veraltete Sätze nach `stop()` werden übersprungen)
    Say(String, Option<Done>, u64),
    Configure(TtsSettings),
}

/// Abbruch-Signal für die laufende Ausgabe.
#[derive(Default)]
struct Stop {
    stopped: Mutex<bool>,
    cv: Condvar,
}

pub struct Speaker {
    tx: Option<Sender<Cmd>>,
    speaking: Arc<AtomicBool>,
    stop: Arc<Stop>,
    generation: Arc<std::sync::atomic::AtomicU64>,
}

impl Speaker {
    /// Startet den TTS-Thread (auf Nicht-Windows ein stummer Platzhalter).
    pub fn new() -> Self {
        let (tx, rx) = channel::<Cmd>();
        let speaking = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(Stop::default());
        let (sp, st) = (speaking.clone(), stop.clone());
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let gen = generation.clone();
        let spawned = std::thread::Builder::new().name("panco-tts".into()).spawn(move || {
            let mut out = Output::default();
            loop {
                // Ohne Auftrag: nach der Haltezeit die natürliche Stimme beenden (Speicher frei).
                let cmd = match rx.recv_timeout(out.keep()) {
                    Ok(cmd) => cmd,
                    Err(RecvTimeoutError::Timeout) => {
                        out.piper = None;
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                match cmd {
                    Cmd::Configure(s) => out.configure(s),
                    Cmd::Say(text, done, g) if g != gen.load(Ordering::SeqCst) => {
                        // nach stop() eingereiht, aber schon überholt
                        let _ = text;
                        if let Some(done) = done {
                            done();
                        }
                    }
                    Cmd::Say(text, done, _) => {
                        *st.stopped.lock().unwrap() = false;
                        sp.store(true, Ordering::Relaxed);
                        out.say(&text, &st);
                        sp.store(false, Ordering::Relaxed);
                        if let Some(done) = done {
                            done();
                        }
                    }
                }
            }
        });
        Self { tx: spawned.ok().map(|_| tx), speaking, stop, generation }
    }

    pub fn configure(&self, settings: TtsSettings) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Cmd::Configure(settings));
        }
    }

    /// Spricht `text`; `done` läuft danach (auch nach Abbruch).
    pub fn say(&self, text: &str, done: Option<Done>) {
        match &self.tx {
            Some(tx) => {
                // Neuer Satz unterbricht den alten (und alles, was noch wartet).
                self.stop();
                let _ = tx.send(Cmd::Say(text.to_string(), done, self.generation.load(Ordering::SeqCst)));
            }
            None => {
                if let Some(d) = done {
                    d()
                }
            }
        }
    }

    /// Hinten anstellen, ohne Laufendes zu unterbrechen (Claude: Satz für Satz, während er schreibt).
    pub fn queue(&self, text: &str, done: Option<Done>) {
        match &self.tx {
            Some(tx) => {
                let _ = tx.send(Cmd::Say(text.to_string(), done, self.generation.load(Ordering::SeqCst)));
            }
            None => {
                if let Some(d) = done {
                    d()
                }
            }
        }
    }

    /// Bricht die laufende Ausgabe sofort ab – auch alles, was noch in der Warteschlange steht.
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if self.speaking.load(Ordering::Relaxed) {
            *self.stop.stopped.lock().unwrap() = true;
            self.stop.cv.notify_all();
        }
    }

    pub fn is_speaking(&self) -> bool {
        self.speaking.load(Ordering::Relaxed)
    }
}

/// Zustand des TTS-Threads.
#[derive(Default)]
struct Output {
    settings: TtsSettings,
    /// Windows-Stimme, erst beim ersten Gebrauch erzeugt.
    windows: Option<imp::Voice>,
    piper: Option<Piper>,
    /// WAV im Speicher, solange die Wiedergabe daraus liest.
    playing: Option<Vec<u8>>,
}

impl Output {
    fn keep(&self) -> Duration {
        match (&self.piper, &self.settings.natural) {
            (Some(_), Some(n)) => Duration::from_secs(n.keep_secs.max(5)),
            _ => Duration::from_secs(3600),
        }
    }

    fn configure(&mut self, s: TtsSettings) {
        if s.natural != self.settings.natural {
            self.piper = None;
        }
        if let Some(v) = self.windows.as_mut() {
            v.configure(&s);
        }
        self.settings = s;
    }

    /// Spielt eine WAV ab und wartet, bis sie zu Ende ist oder abgebrochen wird.
    /// `next` läuft währenddessen (nächsten Satz erzeugen). Liefert `false` nach Abbruch.
    fn play<T>(&mut self, wav: Vec<u8>, stop: &Stop, next: impl FnOnce(&mut Self) -> T) -> (bool, T) {
        let len = wav_duration(&wav).unwrap_or_default();
        let started = std::time::Instant::now();
        if let Err(e) = imp::play(&wav) {
            eprintln!("[panco] Wiedergabe: {e}");
        }
        self.playing = Some(wav);
        let upcoming = next(self);
        let rest = (len + Duration::from_millis(120)).saturating_sub(started.elapsed());
        let guard = stop.stopped.lock().unwrap();
        let (guard, _) = stop.cv.wait_timeout_while(guard, rest, |s| !*s).unwrap();
        let stopped = *guard;
        drop(guard);
        // Erst stoppen, dann den Puffer freigeben (die Wiedergabe liest daraus).
        imp::stop();
        self.playing = None;
        (!stopped, upcoming)
    }

    fn say(&mut self, text: &str, stop: &Stop) {
        if self.settings.natural.is_some() {
            match self.say_natural(text, stop) {
                Ok(()) => return,
                Err(e) => {
                    // Rückfall: Windows-Stimme, damit Panco nie stumm bleibt.
                    eprintln!("[panco] natürliche Stimme: {e} – nehme die Windows-Stimme");
                    self.piper = None;
                    if *stop.stopped.lock().unwrap() {
                        return;
                    }
                }
            }
        }
        if self.windows.is_none() {
            self.windows = imp::Voice::new();
            if let Some(v) = self.windows.as_mut() {
                v.configure(&self.settings);
            }
        }
        let Some(v) = self.windows.as_mut() else { return };
        match v.synth(text) {
            Ok(wav) => {
                self.play(wav, stop, |_| ());
            }
            Err(e) => eprintln!("[panco] Sprachausgabe: {e}"),
        }
    }

    fn say_natural(&mut self, text: &str, stop: &Stop) -> Result<(), String> {
        let n = self.settings.natural.clone().ok_or("aus")?;
        if self.piper.as_mut().is_some_and(|p| !p.alive()) {
            self.piper = None;
        }
        if self.piper.is_none() {
            self.piper = Some(Piper::start(&n.exe, &n.model, n.pitch, n.rate)?);
        }
        let parts = natural::sentences(text);
        let mut iter = parts.iter();
        let Some(first) = iter.next() else { return Ok(()) };
        let mut wav = self.piper.as_mut().ok_or("aus")?.synth(first)?;
        loop {
            let following = iter.next();
            let (go_on, upcoming) = self.play(wav, stop, |me| following.map(|s| me.piper.as_mut().map(|p| p.synth(s))));
            match upcoming {
                Some(Some(Ok(next))) if go_on => wav = next,
                Some(Some(Err(e))) if go_on => {
                    // Mitten im Text nicht von vorn mit der anderen Stimme anfangen.
                    eprintln!("[panco] natürliche Stimme: {e}");
                    self.piper = None;
                    return Ok(());
                }
                _ => return Ok(()),
            }
        }
    }
}

impl Default for Speaker {
    fn default() -> Self {
        Self::new()
    }
}

/// Installierte Stimmen, deutsche zuerst.
pub fn voices() -> Vec<VoiceInfo> {
    let mut v = imp::voices();
    v.sort_by_key(|v| (!v.language.starts_with("de"), !v.female, v.name.clone()));
    v
}

/// Dauer einer WAV-Datei aus dem Kopf (Bytes pro Sekunde + Größe des Datenblocks).
fn wav_duration(wav: &[u8]) -> Option<Duration> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let (mut pos, mut byte_rate, mut data) = (12usize, 0u32, 0u32);
    while pos + 8 <= wav.len() {
        let id = &wav[pos..pos + 4];
        let size = u32::from_le_bytes(wav[pos + 4..pos + 8].try_into().ok()?);
        if id == b"fmt " && pos + 20 <= wav.len() {
            byte_rate = u32::from_le_bytes(wav[pos + 16..pos + 20].try_into().ok()?);
        } else if id == b"data" {
            data = size.min((wav.len() - pos - 8) as u32);
            break;
        }
        pos += 8 + size as usize + (size as usize & 1);
    }
    (byte_rate > 0).then(|| Duration::from_secs_f64(data as f64 / byte_rate as f64))
}

#[cfg(windows)]
mod imp {
    use super::{TtsSettings, VoiceInfo};
    use windows::core::{HSTRING, PCWSTR};
    use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceGender, VoiceInformation};
    use windows::Storage::Streams::DataReader;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FLAGS, SND_MEMORY, SND_NODEFAULT};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    pub struct Voice {
        synth: SpeechSynthesizer,
    }

    fn all() -> Vec<VoiceInformation> {
        let Ok(list) = SpeechSynthesizer::AllVoices() else { return vec![] };
        (0..list.Size().unwrap_or(0)).filter_map(|i| list.GetAt(i).ok()).collect()
    }

    fn info(v: &VoiceInformation) -> VoiceInfo {
        VoiceInfo {
            name: v.DisplayName().map(|s| s.to_string()).unwrap_or_default(),
            language: v.Language().map(|s| s.to_string()).unwrap_or_default(),
            female: v.Gender().map(|g| g == VoiceGender::Female).unwrap_or(false),
        }
    }

    pub fn voices() -> Vec<VoiceInfo> {
        all().iter().map(info).collect()
    }

    /// Gewünschte Stimme oder automatisch: deutsche Frauenstimme (Katja bevorzugt).
    fn pick(name: &str) -> Option<VoiceInformation> {
        let voices = all();
        let by = |f: &dyn Fn(&VoiceInfo) -> bool| voices.iter().find(|v| f(&info(v))).cloned();
        if !name.is_empty() {
            if let Some(v) = by(&|i| i.name == name) {
                return Some(v);
            }
        }
        by(&|i| i.language.starts_with("de") && i.name.contains("Katja"))
            .or_else(|| by(&|i| i.language.starts_with("de") && i.female))
            .or_else(|| by(&|i| i.language.starts_with("de")))
    }

    impl Voice {
        pub fn new() -> Option<Self> {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            let synth = SpeechSynthesizer::new().ok()?;
            let mut v = Voice { synth };
            v.configure(&TtsSettings::default());
            Some(v)
        }

        pub fn configure(&mut self, s: &TtsSettings) {
            if let Some(v) = pick(&s.voice) {
                let _ = self.synth.SetVoice(&v);
            }
            if let Ok(o) = self.synth.Options() {
                let _ = o.SetAudioPitch(s.pitch.clamp(0.5, 2.0) as f64);
                let _ = o.SetSpeakingRate(s.rate.clamp(0.5, 2.0) as f64);
            }
        }

        /// Erzeugt den Text als WAV im Speicher.
        pub fn synth(&mut self, text: &str) -> Result<Vec<u8>, String> {
            let err = |e: windows::core::Error| e.message().to_string();
            let stream =
                self.synth.SynthesizeTextToStreamAsync(&HSTRING::from(text)).map_err(err)?.join().map_err(err)?;
            let size = stream.Size().map_err(err)? as u32;
            let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).map_err(err)?).map_err(err)?;
            reader.LoadAsync(size).map_err(err)?.join().map_err(err)?;
            let mut wav = vec![0u8; size as usize];
            reader.ReadBytes(&mut wav).map_err(err)?;
            Ok(wav)
        }
    }

    /// Startet die Wiedergabe (asynchron). Der Puffer muss bis `stop()` leben.
    pub fn play(wav: &[u8]) -> Result<(), String> {
        let ok =
            unsafe { PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT) };
        if ok.as_bool() {
            Ok(())
        } else {
            Err("Wiedergabe nicht möglich".into())
        }
    }

    /// Stoppt die Wiedergabe (synchron).
    pub fn stop() {
        unsafe {
            let _ = PlaySoundW(PCWSTR::null(), None, SND_FLAGS(0));
        }
    }
}

/// Entwicklung unter Linux: keine Windows-Stimme, keine Wiedergabe (die natürliche Stimme wird
/// trotzdem erzeugt, damit sich Tempo und Speicher messen lassen).
#[cfg(not(windows))]
mod imp {
    use super::{TtsSettings, VoiceInfo};

    pub struct Voice;
    impl Voice {
        pub fn new() -> Option<Self> {
            None
        }
        pub fn configure(&mut self, _: &TtsSettings) {}
        pub fn synth(&mut self, _: &str) -> Result<Vec<u8>, String> {
            Err("keine Windows-Stimme".into())
        }
    }

    pub fn voices() -> Vec<VoiceInfo> {
        vec![]
    }
    pub fn play(_: &[u8]) -> Result<(), String> {
        Ok(())
    }
    pub fn stop() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Natürliche Stimme Satz für Satz; danach Rückmeldung, Stimme bleibt bis zur Haltezeit geladen.
    #[cfg(unix)]
    #[test]
    fn speaks_sentences_with_natural_voice() {
        let (dir, exe, model) = crate::natural::tests::fake_piper("speaker");
        let speaker = Speaker::new();
        speaker.configure(TtsSettings {
            natural: Some(Natural { exe, model, pitch: 1.0, rate: 1.0, keep_secs: 5 }),
            ..TtsSettings::default()
        });
        let (tx, rx) = channel();
        let started = std::time::Instant::now();
        speaker.say(
            "Das ist der erste lange Satz. Und hier kommt der zweite Satz. Und noch ein dritter Satz!",
            Some(Box::new(move || tx.send(()).unwrap())),
        );
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // drei Sätze à 0,1 s (+ kleine Pause) – kein Abbruch, kein Rückfall
        assert!(started.elapsed() >= Duration::from_millis(300), "{:?}", started.elapsed());
        assert!(!speaker.is_speaking());

        // Abbruch mitten im Text
        let (tx, rx) = channel();
        speaker.say(
            "Erster Satz zum Abbrechen hier. Zweiter Satz zum Abbrechen.",
            Some(Box::new(move || tx.send(()).unwrap())),
        );
        speaker.stop();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wav_length_from_header() {
        // 16 kHz, 16 bit, mono → 32000 Bytes/s; 16000 Bytes Daten = 0,5 s
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36u32 + 16_000).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&32_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.resize(wav.len() + 16_000, 0);
        assert_eq!(wav_duration(&wav).unwrap().as_millis(), 500);
        assert!(wav_duration(b"nope").is_none());
    }
}
