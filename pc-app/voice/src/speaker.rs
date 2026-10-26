//! Einfache Sprecherprüfung (optional): Aus den Wake-Word-Proben entsteht ein Stimmprofil –
//! Mittelwert und Streuung von 12 MFCC-Merkmalen, 24 Zahlen, keine Audiodaten. Bei "Panco" wird
//! derselbe Fingerabdruck berechnet und mit dem Profil verglichen.
//!
//! Das ist bewusst leicht (nur beim Wake-Word, einige Millisekunden) und unterscheidet vor allem
//! deutlich andere Stimmen (andere Person, Stimme aus Video/Stream). Es ist kein Sicherheitsmerkmal
//! im Sinne einer Biometrie – nur ein zusätzlicher Filter gegen Fehlaktivierungen.

use realfft::RealFftPlanner;
use serde::{Deserialize, Serialize};
use std::path::Path;

const RATE: f32 = 16_000.0;
const WIN: usize = 400; // 25 ms
const HOP: usize = 160; // 10 ms
const NFFT: usize = 512;
const MELS: usize = 26;
const CEPS: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoiceProfile {
    /// Mittelpunkt der Proben (24 Werte: MFCC-Mittel + MFCC-Streuung)
    pub center: Vec<f32>,
    /// Typischer Abstand der eigenen Proben zum Mittelpunkt
    pub spread: f32,
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10f32.powf(mel / 2595.0) - 1.0)
}

/// Dreieckige Mel-Filter (100 Hz … 7,6 kHz) über dem 512er-Spektrum.
fn mel_bank() -> Vec<Vec<(usize, f32)>> {
    let (lo, hi) = (hz_to_mel(100.0), hz_to_mel(7600.0));
    let points: Vec<f32> = (0..MELS + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f32 / (MELS + 1) as f32)).collect();
    let bin = |hz: f32| hz / RATE * NFFT as f32;
    (0..MELS)
        .map(|m| {
            let (a, b, c) = (bin(points[m]), bin(points[m + 1]), bin(points[m + 2]));
            (a.floor() as usize..=c.ceil() as usize)
                .filter(|&k| k <= NFFT / 2)
                .filter_map(|k| {
                    let k_f = k as f32;
                    let w = if k_f < b { (k_f - a) / (b - a) } else { (c - k_f) / (c - b) };
                    (w > 0.0).then_some((k, w))
                })
                .collect()
        })
        .collect()
}

/// Fingerabdruck einer Äußerung (16 kHz Mono): Mittel und Streuung von MFCC 1–12 über die
/// lauteren Frames (Stille und leise Ränder zählen nicht).
pub fn fingerprint(pcm: &[f32]) -> Option<Vec<f32>> {
    if pcm.len() < WIN * 10 {
        return None;
    }
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(NFFT);
    let bank = mel_bank();
    let hamming: Vec<f32> =
        (0..WIN).map(|i| 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / (WIN - 1) as f32).cos()).collect();
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    let mut frames: Vec<(f32, [f32; CEPS])> = Vec::new();
    let mut start = 0;
    while start + WIN <= pcm.len() {
        let frame = &pcm[start..start + WIN];
        let energy = frame.iter().map(|v| v * v).sum::<f32>() / WIN as f32;
        input.iter_mut().for_each(|v| *v = 0.0);
        // Vorbetonung (Höhen anheben) + Fenster
        for i in 0..WIN {
            let prev = if i == 0 { 0.0 } else { frame[i - 1] };
            input[i] = (frame[i] - 0.97 * prev) * hamming[i];
        }
        fft.process(&mut input, &mut spectrum).ok()?;
        let power: Vec<f32> = spectrum.iter().map(|c| c.norm_sqr()).collect();
        let logmel: Vec<f32> =
            bank.iter().map(|f| (f.iter().map(|&(k, w)| power[k] * w).sum::<f32>() + 1e-10).ln()).collect();
        let mut ceps = [0f32; CEPS];
        for (n, c) in ceps.iter_mut().enumerate() {
            let n = n + 1; // c0 (Lautstärke) weglassen
            *c = logmel
                .iter()
                .enumerate()
                .map(|(m, v)| v * (std::f32::consts::PI * n as f32 * (m as f32 + 0.5) / MELS as f32).cos())
                .sum();
        }
        frames.push((energy, ceps));
        start += HOP;
    }
    // Nur die lautere Hälfte der Frames (dort steckt die Stimme)
    let mut energies: Vec<f32> = frames.iter().map(|f| f.0).collect();
    energies.sort_by(f32::total_cmp);
    let cut = energies[energies.len() / 2];
    let voiced: Vec<&[f32; CEPS]> = frames.iter().filter(|f| f.0 >= cut && f.0 > 1e-6).map(|f| &f.1).collect();
    if voiced.len() < 10 {
        return None;
    }
    let n = voiced.len() as f32;
    let mut out = vec![0f32; CEPS * 2];
    for c in 0..CEPS {
        let mean = voiced.iter().map(|f| f[c]).sum::<f32>() / n;
        let var = voiced.iter().map(|f| (f[c] - mean).powi(2)).sum::<f32>() / n;
        out[c] = mean;
        out[CEPS + c] = var.sqrt();
    }
    Some(out)
}

/// Abstand zweier Fingerabdrücke; höhere Koeffizienten sind kleiner, daher gewichtet (Liftering).
fn distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .enumerate()
        .map(|(i, (x, y))| {
            let n = (i % CEPS) as f32 + 1.0;
            let w = 1.0 + 0.5 * n; // höhere Koeffizienten aufwerten
            (w * (x - y)).powi(2)
        })
        .sum::<f32>()
        .sqrt()
}

impl VoiceProfile {
    /// Profil aus den Wake-Word-Proben (mindestens 3).
    pub fn build(samples: &[Vec<f32>]) -> Option<Self> {
        let prints: Vec<Vec<f32>> = samples.iter().filter_map(|s| fingerprint(s)).collect();
        if prints.len() < 3 {
            return None;
        }
        let dims = prints[0].len();
        let center: Vec<f32> =
            (0..dims).map(|d| prints.iter().map(|p| p[d]).sum::<f32>() / prints.len() as f32).collect();
        let spread = prints.iter().map(|p| distance(p, &center)).sum::<f32>() / prints.len() as f32;
        Some(Self { center, spread: spread.max(1e-3) })
    }

    /// 0 = genau wie die eigenen Proben, ~1 = so weit weg wie die eigenen Proben im Schnitt.
    pub fn score(&self, pcm: &[f32]) -> Option<f32> {
        fingerprint(pcm).map(|p| distance(&p, &self.center) / self.spread)
    }

    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, serde_json::to_vec(self).unwrap_or_default()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(f0: f32, ms: usize) -> Vec<f32> {
        // Stimmähnlich: Grundton + Obertöne mit Formant-Gewichtung
        (0..16 * ms)
            .map(|i| {
                let t = i as f32 / RATE;
                (1..12).map(|h| (2.0 * std::f32::consts::PI * f0 * h as f32 * t).sin() / h as f32).sum::<f32>() * 0.2
            })
            .collect()
    }

    #[test]
    fn same_voice_close_other_voice_far() {
        let own: Vec<Vec<f32>> = (0..5).map(|i| tone(120.0 + i as f32 * 2.0, 600)).collect();
        let p = VoiceProfile::build(&own).unwrap();
        let same = p.score(&tone(123.0, 600)).unwrap();
        let other = p.score(&tone(240.0, 600)).unwrap();
        assert!(same < other, "{same} vs {other}");
        assert!(fingerprint(&[0.0; 100]).is_none());
    }
}
