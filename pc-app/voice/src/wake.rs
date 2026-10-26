//! Wake-Word "Panco" mit rustpotter (reines Rust, komplett offline).
//!
//! Panco lernt das Wort aus 6 eigenen Aufnahmen ("Panco einsprechen"). Gespeichert wird nur die
//! daraus berechnete Merkmalsdatei (MFCC-Vorlagen, `panco.rpw`) – keine Audiodaten.

use crate::wav;
use rustpotter::{
    DetectorConfig, Rustpotter, RustpotterConfig, ScoreMode, VADMode, WakewordRef, WakewordRefBuildFromBuffers,
    WakewordSave,
};
use std::collections::HashMap;
use std::path::Path;

pub const SAMPLES_NEEDED: usize = 6;
const MFCC_SIZE: u16 = 16;

pub struct WakeDetector {
    rp: Rustpotter,
    frame: usize,
    buf: Vec<f32>,
}

fn config(threshold: f32) -> RustpotterConfig {
    RustpotterConfig {
        detector: DetectorConfig {
            threshold,
            avg_threshold: 0.2,
            min_scores: 5,
            score_mode: ScoreMode::Max,
            // Eingebaute VAD: rechnet bei Stille keine Vergleiche (CPU sparen).
            vad_mode: Some(VADMode::Easy),
            ..Default::default()
        },
        ..Default::default()
    }
}

impl WakeDetector {
    pub fn load(path: &Path, threshold: f32) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("Wake-Word-Datei nicht lesbar: {e}"))?;
        let mut rp = Rustpotter::new(&config(threshold))?;
        rp.add_wakeword_from_buffer("panco", &bytes)?;
        let frame = rp.get_samples_per_frame();
        Ok(Self { rp, frame, buf: Vec::with_capacity(frame * 2) })
    }

    /// 16-kHz-Samples einspeisen. Liefert die Erkennungs-Punktzahl, wenn "Panco" erkannt wurde.
    pub fn process(&mut self, samples: &[f32]) -> Option<f32> {
        let mut hit = None;
        self.buf.extend_from_slice(samples);
        let mut start = 0;
        while self.buf.len() - start >= self.frame {
            let chunk = self.buf[start..start + self.frame].to_vec();
            start += self.frame;
            if let Some(d) = self.rp.process_samples(chunk) {
                hit = Some(d.score);
            }
        }
        self.buf.drain(..start);
        hit
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.rp.reset();
    }
}

/// Baut aus den Aufnahmen (16 kHz) die Merkmalsdatei und speichert sie.
pub fn train(samples: &[Vec<f32>], threshold: f32, path: &Path) -> Result<(), String> {
    if samples.len() < 3 {
        return Err("Zu wenige Aufnahmen".into());
    }
    let buffers: HashMap<String, Vec<u8>> =
        samples.iter().enumerate().map(|(i, s)| (format!("sample{i}.wav"), wav::encode(s, 16_000))).collect();
    let wakeword =
        WakewordRef::new_from_sample_buffers("panco".into(), Some(threshold), Some(0.2), buffers, MFCC_SIZE)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    wakeword.save_to_file(&path.to_string_lossy())
}

/// Schneidet Stille am Anfang und Ende ab (für saubere Trainingsaufnahmen).
pub fn trim_silence(samples: &[f32]) -> Vec<f32> {
    const WIN: usize = 160; // 10 ms
    let loud = |w: &[f32]| (w.iter().map(|v| v * v).sum::<f32>() / w.len() as f32).sqrt() > 0.01;
    let windows: Vec<bool> = samples.chunks(WIN).map(loud).collect();
    let (Some(first), Some(last)) = (windows.iter().position(|l| *l), windows.iter().rposition(|l| *l)) else {
        return Vec::new();
    };
    // 50 ms Rand lassen
    let start = first.saturating_sub(5) * WIN;
    let end = ((last + 6) * WIN).min(samples.len());
    samples[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims() {
        let mut x = vec![0.0; 1600];
        x.extend(vec![0.5; 1600]);
        x.extend(vec![0.0; 3200]);
        let t = trim_silence(&x);
        assert!(t.len() >= 1600 && t.len() <= 1600 + 2 * 1760, "{}", t.len());
        assert!(trim_silence(&[0.0; 800]).is_empty());
    }
}
