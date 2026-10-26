//! Einfache Sprachaktivitätserkennung (Energie mit adaptivem Grundrauschen).
//! Dient zwei Zwecken: Wake-Word nur bei Geräuschen rechnen lassen (CPU sparen) und
//! das Ende eines Befehls erkennen (Stille nach Sprache).

/// 20 ms bei 16 kHz.
pub const FRAME: usize = 320;
const MIN_THRESHOLD: f32 = 0.006;
const FACTOR: f32 = 3.5;
/// Nach dem letzten lauten Frame noch 300 ms als Sprache werten (Wortpausen).
const HANGOVER_FRAMES: u32 = 15;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub speech: bool,
    /// Pegel 0..1 (logarithmisch, für die Schallwellen-Anzeige)
    pub level: f32,
}

pub struct Vad {
    floor: f32,
    hang: u32,
}

impl Default for Vad {
    fn default() -> Self {
        Self { floor: 0.002, hang: 0 }
    }
}

impl Vad {
    /// Bewertet genau einen Frame mit `FRAME` Samples.
    pub fn frame(&mut self, samples: &[f32]) -> Frame {
        let rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len().max(1) as f32).sqrt();
        let threshold = (self.floor * FACTOR).max(MIN_THRESHOLD);
        let speech = if rms > threshold {
            self.hang = HANGOVER_FRAMES;
            true
        } else if self.hang > 0 {
            self.hang -= 1;
            true
        } else {
            // Grundrauschen nur in Ruhe nachführen, nach oben begrenzt (Lüfter ja, Musik nein).
            self.floor = (self.floor * 0.97 + rms * 0.03).min(0.03);
            false
        };
        let level = ((20.0 * rms.max(1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0);
        Frame { speech, level }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(amp: f32, seed: &mut u32) -> Vec<f32> {
        (0..FRAME)
            .map(|_| {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (*seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    #[test]
    fn detects_speech_and_hangover() {
        let mut vad = Vad::default();
        let mut seed = 1;
        for _ in 0..100 {
            assert!(!vad.frame(&noise(0.002, &mut seed)).speech, "leises Rauschen ist keine Sprache");
        }
        assert!(vad.frame(&noise(0.2, &mut seed)).speech, "laut = Sprache");
        // Hangover: noch 15 Frames Sprache, danach Ruhe
        let after: Vec<bool> = (0..20).map(|_| vad.frame(&noise(0.002, &mut seed)).speech).collect();
        assert!(after[..15].iter().all(|s| *s));
        assert!(!after[19]);
    }

    #[test]
    fn adapts_to_fan_noise() {
        let mut vad = Vad::default();
        let mut seed = 7;
        // Konstantes Lüfterrauschen über der Mindestschwelle
        let early: Vec<bool> = (0..300).map(|_| vad.frame(&noise(0.012, &mut seed)).speech).collect();
        assert!(!early[250..].iter().any(|s| *s), "Grundrauschen wird gelernt");
        assert!(vad.frame(&noise(0.3, &mut seed)).speech);
    }
}
