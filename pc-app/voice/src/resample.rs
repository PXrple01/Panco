//! Streaming-Resampler: beliebige Mikrofonrate (meist 48 kHz) → 16 kHz Mono für Wake-Word/Whisper.
//! Tiefpass (gefensterter Sinc, 33 Taps) gegen Aliasing, dann lineare Interpolation.
//! Kosten bei 48 kHz: ~1,6 Mio. Multiplikationen/s – vernachlässigbar.

pub const TARGET_RATE: u32 = 16_000;
const TAPS: usize = 33;

pub struct Resampler {
    /// Eingangssamples pro Ausgangssample (48 kHz → 3.0).
    step: f64,
    /// Position des nächsten Ausgangssamples relativ zum vorigen Eingangssample (0..=1].
    t: f64,
    taps: Vec<f32>,
    delay: [f32; TAPS],
    pos: usize,
    prev: f32,
    passthrough: bool,
}

impl Resampler {
    pub fn new(in_rate: u32) -> Self {
        let passthrough = in_rate == TARGET_RATE;
        // Grenzfrequenz knapp unter der neuen Nyquist-Frequenz (7,2 kHz bei 16 kHz Ziel).
        let fc = 7_200.0 / in_rate as f64;
        let m = (TAPS - 1) as f64;
        let mut taps: Vec<f32> = (0..TAPS)
            .map(|i| {
                let x = i as f64 - m / 2.0;
                let sinc = if x == 0.0 {
                    2.0 * fc
                } else {
                    (2.0 * std::f64::consts::PI * fc * x).sin() / (std::f64::consts::PI * x)
                };
                let hamming = 0.54 - 0.46 * (2.0 * std::f64::consts::PI * i as f64 / m).cos();
                (sinc * hamming) as f32
            })
            .collect();
        let sum: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= sum);
        Self {
            step: in_rate as f64 / TARGET_RATE as f64,
            t: 1.0,
            taps,
            delay: [0.0; TAPS],
            pos: 0,
            prev: 0.0,
            passthrough,
        }
    }

    /// Hängt die 16-kHz-Ausgabe an `out` an.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.passthrough {
            out.extend_from_slice(input);
            return;
        }
        for &x in input {
            self.delay[self.pos] = x;
            self.pos = (self.pos + 1) % TAPS;
            let mut y = 0.0f32;
            for (i, tap) in self.taps.iter().enumerate() {
                y += tap * self.delay[(self.pos + i) % TAPS];
            }
            while self.t <= 1.0 {
                out.push(self.prev + (y - self.prev) * self.t as f32);
                self.t += self.step;
            }
            self.t -= 1.0;
            self.prev = y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, secs: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn length_and_passband() {
        for rate in [48_000, 44_100, 32_000] {
            let mut r = Resampler::new(rate);
            let mut out = Vec::new();
            // In kleinen Stücken wie aus dem Audio-Callback
            for chunk in sine(440.0, rate, 1.0).chunks(480) {
                r.process(chunk, &mut out);
            }
            assert!((out.len() as i64 - 16_000).abs() <= 2, "{rate}: {} Samples", out.len());
            let level = rms(&out[1000..]);
            assert!((level - 0.707).abs() < 0.03, "{rate}: Pegel {level}");
        }
    }

    #[test]
    fn blocks_aliasing() {
        // 12 kHz würde bei 16 kHz zu 4 kHz zurückfalten – muss stark gedämpft sein.
        let mut r = Resampler::new(48_000);
        let mut out = Vec::new();
        r.process(&sine(12_000.0, 48_000, 0.5), &mut out);
        assert!(rms(&out[200..]) < 0.05, "Aliasing {}", rms(&out[200..]));
    }

    #[test]
    fn passthrough_at_16k() {
        let mut r = Resampler::new(16_000);
        let mut out = Vec::new();
        r.process(&[0.1, 0.2, 0.3], &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }
}
