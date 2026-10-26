//! Kurzes Sprachfeedback über die Windows-Sprachausgabe (SAPI, lokal, keine Cloud).
//! Läuft in einem eigenen Thread; neue Sätze unterbrechen alte.

use std::sync::mpsc::{channel, Sender};

pub type Done = Box<dyn FnOnce() + Send>;

pub struct Speaker {
    tx: Option<Sender<(String, Option<Done>)>>,
}

impl Speaker {
    /// Startet den TTS-Thread (auf Nicht-Windows ein stummer Platzhalter).
    pub fn new() -> Self {
        let (tx, rx) = channel::<(String, Option<Done>)>();
        let spawned = std::thread::Builder::new().name("panco-tts".into()).spawn(move || {
            let mut voice = imp::Voice::new();
            for (text, done) in rx {
                if let Some(v) = voice.as_mut() {
                    v.speak(&text);
                }
                if let Some(done) = done {
                    done();
                }
            }
        });
        Self { tx: spawned.ok().map(|_| tx) }
    }

    /// Spricht `text`; `done` läuft danach (z. B. um auf eine Ja/Nein-Antwort zu hören).
    pub fn say(&self, text: &str, done: Option<Done>) {
        match &self.tx {
            Some(tx) => {
                let _ = tx.send((text.to_string(), done));
            }
            None => {
                if let Some(d) = done {
                    d()
                }
            }
        }
    }
}

impl Default for Speaker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
mod imp {
    use windows::core::HSTRING;
    use windows::Win32::Media::Speech::{ISpVoice, SpVoice, SPF_DEFAULT, SPF_IS_NOT_XML, SPF_PURGEBEFORESPEAK};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

    pub struct Voice(ISpVoice);

    impl Voice {
        pub fn new() -> Option<Self> {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let v: ISpVoice = CoCreateInstance(&SpVoice, None, CLSCTX_ALL).ok()?;
                let _ = v.SetRate(1);
                Some(Voice(v))
            }
        }

        /// Synchron (blockiert nur den TTS-Thread).
        pub fn speak(&mut self, text: &str) {
            let flags = (SPF_DEFAULT.0 | SPF_PURGEBEFORESPEAK.0 | SPF_IS_NOT_XML.0) as u32;
            unsafe {
                let _ = self.0.Speak(&HSTRING::from(text), flags, None);
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct Voice;
    impl Voice {
        pub fn new() -> Option<Self> {
            None
        }
        pub fn speak(&mut self, _: &str) {}
    }
}
