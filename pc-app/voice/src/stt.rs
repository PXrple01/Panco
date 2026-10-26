//! Spracherkennung. Standard: whisper.cpp (lokal). Das Modell wird nur bei Bedarf geladen und
//! nach `keep_secs` ohne Nutzung wieder freigegeben (siehe pipeline.rs).

use std::path::{Path, PathBuf};
use std::time::Instant;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Debug, Clone)]
pub struct Transcript {
    pub text: String,
    pub engine: &'static str,
    /// Länge der Aufnahme
    pub audio_ms: u64,
    /// Zeit zum Laden des Modells (0 = war schon geladen)
    pub load_ms: u64,
    /// Zeit für die eigentliche Erkennung
    pub stt_ms: u64,
}

/// Typische Whisper-Halluzinationen bei Stille/Rauschen (aus Untertitel-Trainingsdaten).
const HALLUCINATIONS: [&str; 8] = [
    "untertitel",
    "vielen dank fürs zuschauen",
    "vielen dank für's zuschauen",
    "danke fürs zuschauen",
    "copyright",
    "amara.org",
    "swr 20",
    "zdf",
];

pub struct WhisperStt {
    model: PathBuf,
    ctx: Option<WhisperContext>,
    threads: i32,
}

impl WhisperStt {
    pub fn new(model: PathBuf) -> Self {
        // Halbe Kernzahl, max. 4: schnell genug, ohne ein laufendes Spiel auszubremsen.
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        Self { model, ctx: None, threads: (cores / 2).clamp(2, 4) as i32 }
    }

    pub fn set_model(&mut self, model: PathBuf) {
        if model != self.model {
            self.model = model;
            self.ctx = None;
        }
    }

    pub fn model(&self) -> &Path {
        &self.model
    }

    pub fn loaded(&self) -> bool {
        self.ctx.is_some()
    }

    /// Gibt das Modell frei (RAM zurück an das System).
    pub fn unload(&mut self) {
        self.ctx = None;
    }

    /// `pcm`: 16 kHz Mono. `prompt`: Vokabular-Hinweis (Spielnamen, Befehlswörter).
    pub fn transcribe(&mut self, pcm: &[f32], prompt: &str) -> Result<Transcript, String> {
        let mut load_ms = 0;
        if self.ctx.is_none() {
            if !self.model.is_file() {
                return Err("Sprachmodell fehlt – bitte in den Einstellungen herunterladen".into());
            }
            let t = Instant::now();
            let ctx = WhisperContext::new_with_params(&self.model, WhisperContextParameters::default())
                .map_err(|e| format!("Modell nicht ladbar: {e}"))?;
            load_ms = t.elapsed().as_millis() as u64;
            self.ctx = Some(ctx);
        }
        let ctx = self.ctx.as_ref().unwrap();
        let t = Instant::now();

        // Whisper braucht mindestens ~1 s Audio.
        let mut audio = pcm.to_vec();
        if audio.len() < 16_000 + 1_600 {
            audio.resize(16_000 + 1_600, 0.0);
        }
        let secs = audio.len() as f32 / 16_000.0;

        let mut state = ctx.create_state().map_err(|e| e.to_string())?;
        let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        p.set_language(Some("de"));
        p.set_n_threads(self.threads);
        p.set_translate(false);
        p.set_no_context(true);
        p.set_no_timestamps(true);
        p.set_single_segment(true);
        p.set_suppress_blank(true);
        p.set_print_special(false);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_timestamps(false);
        // Nur einen Teil des 30-s-Fensters rechnen: 512 (~10 s) ist der gemessene Kompromiss –
        // kleiner wird die Erkennung deutlich schlechter, größer kaum besser, aber langsamer.
        p.set_audio_ctx(((secs * 50.0) as i32 + 64).clamp(512, 1500));
        // Befehle sind kurz; begrenzt außerdem Halluzinationsschleifen.
        p.set_max_tokens(64);
        if !prompt.is_empty() {
            p.set_initial_prompt(prompt);
        }
        state.full(p, &audio).map_err(|e| e.to_string())?;

        let mut text = String::new();
        for seg in state.as_iter() {
            if seg.no_speech_probability() > 0.6 {
                continue;
            }
            if let Ok(s) = seg.to_str_lossy() {
                text.push_str(&s);
            }
        }
        let text = clean(&text);
        Ok(Transcript {
            text,
            engine: "whisper",
            audio_ms: (pcm.len() as u64 * 1000) / 16_000,
            load_ms,
            stt_ms: t.elapsed().as_millis() as u64,
        })
    }
}

/// Leerzeichen/Klammern säubern, bekannte Halluzinationen verwerfen.
pub fn clean(text: &str) -> String {
    let t = text.trim();
    // "[Musik]", "(Lachen)", "*Applaus*" – Geräuschbeschreibungen sind kein Befehl.
    let bracketed = |o: char, c: char| t.starts_with(o) && t.ends_with(c);
    if bracketed('[', ']') || bracketed('(', ')') || bracketed('*', '*') {
        return String::new();
    }
    let lower = t.to_lowercase();
    if HALLUCINATIONS.iter().any(|h| lower.contains(h)) || lower.chars().all(|c| !c.is_alphanumeric()) {
        return String::new();
    }
    t.to_string()
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn filters_hallucinations() {
        assert_eq!(clean(" Öffne Deadlock. "), "Öffne Deadlock.");
        assert_eq!(clean("Untertitel im Auftrag des ZDF, 2017"), "");
        assert_eq!(clean("[Musik]"), "");
        assert_eq!(clean("(Lachen)"), "");
        assert_eq!(clean(" ... "), "");
    }
}
