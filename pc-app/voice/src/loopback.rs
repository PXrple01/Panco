//! Systemton-Abgleich: Was gerade aus den Lautsprechern kommt (Spiel, Stream, Video), landet in
//! einem kurzen Ringpuffer im Speicher (3 s, nichts wird gespeichert oder gesendet). Hört das
//! Mikrofon "Panco", prüft die Pipeline, ob das Wort auch im Systemton vorkam – dann kam es aus
//! den Lautsprechern und wird ignoriert.
//!
//! Der Audio-Callback mischt nur auf Mono und schreibt in den Puffer; umgerechnet und geprüft wird
//! erst beim Wake-Word. Spielt nichts, liefert Windows keine Daten (kein Aufwachen).

use crate::resample::Resampler;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const SECONDS: usize = 3;

pub struct Ring {
    rate: u32,
    buf: VecDeque<f32>,
}

impl Ring {
    pub fn new(rate: u32) -> Self {
        Self { rate, buf: VecDeque::with_capacity(rate as usize * SECONDS) }
    }

    pub fn push(&mut self, mono: &[f32]) {
        let cap = self.rate as usize * SECONDS;
        let mono = &mono[mono.len().saturating_sub(cap)..];
        let excess = (self.buf.len() + mono.len()).saturating_sub(cap);
        self.buf.drain(..excess);
        self.buf.extend(mono.iter().copied());
    }

    /// Die letzten `ms` Millisekunden als 16 kHz Mono.
    pub fn last_16k(&self, ms: usize) -> Vec<f32> {
        let take = (self.rate as usize * ms / 1000).min(self.buf.len());
        let raw: Vec<f32> = self.buf.iter().skip(self.buf.len() - take).copied().collect();
        let mut out = Vec::with_capacity(16 * ms);
        Resampler::new(self.rate).process(&raw, &mut out);
        out
    }

    /// Lautstärke (RMS) der letzten `ms` Millisekunden.
    pub fn level(&self, ms: usize) -> f32 {
        let take = (self.rate as usize * ms / 1000).min(self.buf.len());
        if take == 0 {
            return 0.0;
        }
        (self.buf.iter().skip(self.buf.len() - take).map(|v| v * v).sum::<f32>() / take as f32).sqrt()
    }
}

pub type Shared = Arc<Mutex<Ring>>;

/// Hält den Systemton-Mitschnitt offen (Windows: WASAPI-Loopback des Standard-Ausgabegeräts).
pub struct Loopback {
    #[cfg(windows)]
    _stream: cpal::Stream,
    pub ring: Shared,
}

#[cfg(windows)]
pub fn open() -> Result<Loopback, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("Kein Ausgabegerät")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!("Systemton-Format {:?} nicht unterstützt", supported.sample_format()));
    }
    let ring: Shared = Arc::new(Mutex::new(Ring::new(rate)));
    let r = ring.clone();
    let stream = device
        .build_input_stream(
            supported.config(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let mono: Vec<f32> = data.chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect();
                if let Ok(mut ring) = r.lock() {
                    ring.push(&mono);
                }
            },
            |e| eprintln!("[panco] Systemton: {e}"),
            None,
        )
        .map_err(|e| format!("Systemton nicht abgreifbar: {e}"))?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Loopback { _stream: stream, ring })
}

/// Unter Linux (Entwicklung) gibt es keinen Loopback; Tests speisen den Puffer selbst.
#[cfg(not(windows))]
pub fn open() -> Result<Loopback, String> {
    Err("Systemton-Abgleich nur unter Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_last_seconds() {
        let mut r = Ring::new(48_000);
        r.push(&vec![0.5; 48_000 * 4]);
        assert_eq!(r.buf.len(), 48_000 * SECONDS);
        assert!((r.level(500) - 0.5).abs() < 1e-3);
        let pcm = r.last_16k(1000);
        assert!((pcm.len() as i32 - 16_000).abs() < 50, "{}", pcm.len());
        assert_eq!(Ring::new(16_000).level(100), 0.0);
    }
}
