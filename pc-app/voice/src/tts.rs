//! Sprachausgabe über die Windows-Stimmen (lokal, keine Cloud). Tonhöhe und Tempo sind einstellbar –
//! Panco spricht standardmäßig etwas höher, passend zum Avatar.
//! Läuft in einem eigenen Thread; ein neuer Satz und `stop()` brechen den laufenden sofort ab
//! (Dazwischenreden bei Rückfragen).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
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
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self { voice: String::new(), pitch: 1.0, rate: 1.0 }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VoiceInfo {
    pub name: String,
    pub language: String,
    pub female: bool,
}

enum Cmd {
    Say(String, Option<Done>),
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
}

impl Speaker {
    /// Startet den TTS-Thread (auf Nicht-Windows ein stummer Platzhalter).
    pub fn new() -> Self {
        let (tx, rx) = channel::<Cmd>();
        let speaking = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(Stop::default());
        let (sp, st) = (speaking.clone(), stop.clone());
        let spawned = std::thread::Builder::new().name("panco-tts".into()).spawn(move || {
            let mut voice = imp::Voice::new();
            for cmd in rx {
                match cmd {
                    Cmd::Configure(s) => {
                        if let Some(v) = voice.as_mut() {
                            v.configure(&s);
                        }
                    }
                    Cmd::Say(text, done) => {
                        *st.stopped.lock().unwrap() = false;
                        sp.store(true, Ordering::Relaxed);
                        if let Some(v) = voice.as_mut() {
                            match v.start(&text) {
                                // Warten, bis der Satz zu Ende ist oder abgebrochen wird.
                                Ok(len) => {
                                    let guard = st.stopped.lock().unwrap();
                                    let _ = st.cv.wait_timeout_while(guard, len + Duration::from_millis(250), |s| !*s);
                                }
                                Err(e) => eprintln!("[panco] Sprachausgabe: {e}"),
                            }
                            // Erst stoppen, dann den Puffer freigeben (die Wiedergabe liest daraus).
                            v.finish();
                        }
                        sp.store(false, Ordering::Relaxed);
                        if let Some(done) = done {
                            done();
                        }
                    }
                }
            }
        });
        Self { tx: spawned.ok().map(|_| tx), speaking, stop }
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
                // Neuer Satz unterbricht den alten.
                self.stop();
                let _ = tx.send(Cmd::Say(text.to_string(), done));
            }
            None => {
                if let Some(d) = done {
                    d()
                }
            }
        }
    }

    /// Bricht die laufende Ausgabe sofort ab.
    pub fn stop(&self) {
        if self.speaking.load(Ordering::Relaxed) {
            *self.stop.stopped.lock().unwrap() = true;
            self.stop.cv.notify_all();
        }
    }

    pub fn is_speaking(&self) -> bool {
        self.speaking.load(Ordering::Relaxed)
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
#[cfg_attr(not(windows), allow(dead_code))]
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
    use std::time::Duration;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceGender, VoiceInformation};
    use windows::Storage::Streams::DataReader;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FLAGS, SND_MEMORY, SND_NODEFAULT};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    pub struct Voice {
        synth: SpeechSynthesizer,
        /// WAV im Speicher, solange PlaySound daraus liest.
        playing: Option<Vec<u8>>,
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
            let mut v = Voice { synth, playing: None };
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

        /// Erzeugt den Satz und startet die Wiedergabe. Liefert die Dauer.
        pub fn start(&mut self, text: &str) -> Result<Duration, String> {
            let err = |e: windows::core::Error| e.message().to_string();
            let stream =
                self.synth.SynthesizeTextToStreamAsync(&HSTRING::from(text)).map_err(err)?.join().map_err(err)?;
            let size = stream.Size().map_err(err)? as u32;
            let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).map_err(err)?).map_err(err)?;
            reader.LoadAsync(size).map_err(err)?.join().map_err(err)?;
            let mut wav = vec![0u8; size as usize];
            reader.ReadBytes(&mut wav).map_err(err)?;
            let len = super::wav_duration(&wav).ok_or("unbekanntes Audioformat")?;
            let ok =
                unsafe { PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT) };
            self.playing = Some(wav);
            if !ok.as_bool() {
                return Err("Wiedergabe nicht möglich".into());
            }
            Ok(len)
        }

        /// Stoppt die Wiedergabe (synchron) und gibt den Puffer frei.
        pub fn finish(&mut self) {
            if self.playing.take().is_some() {
                unsafe {
                    let _ = PlaySoundW(PCWSTR::null(), None, SND_FLAGS(0));
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{TtsSettings, VoiceInfo};
    use std::time::Duration;

    pub struct Voice;
    impl Voice {
        pub fn new() -> Option<Self> {
            None
        }
        pub fn configure(&mut self, _: &TtsSettings) {}
        pub fn start(&mut self, _: &str) -> Result<Duration, String> {
            Ok(Duration::ZERO)
        }
        pub fn finish(&mut self) {}
    }

    pub fn voices() -> Vec<VoiceInfo> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::wav_duration;

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
