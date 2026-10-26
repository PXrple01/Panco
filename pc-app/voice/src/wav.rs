//! WAV im Speicher (16 Bit PCM, Mono). Wird nur für das Wake-Word-Training gebraucht –
//! die Bytes landen nie auf der Festplatte.

pub fn encode(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt-Chunk-Größe
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // Mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // Bytes/s
    out.extend_from_slice(&2u16.to_le_bytes()); // Block-Ausrichtung
    out.extend_from_slice(&16u16.to_le_bytes()); // Bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

/// Liest 16-Bit-Mono-WAV (für Tests und Messwerkzeuge).
pub fn decode(bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12;
    let (mut rate, mut channels, mut bits) = (0u32, 0u16, 0u16);
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = bytes.get(pos + 8..(pos + 8 + len).min(bytes.len()))?;
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            if bits != 16 || channels == 0 {
                return None;
            }
            let frames = body.chunks_exact(2 * channels as usize);
            let samples = frames.map(|f| i16::from_le_bytes([f[0], f[1]]) as f32 / i16::MAX as f32).collect::<Vec<_>>();
            return Some((samples, rate));
        }
        pos += 8 + len + (len & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn roundtrip() {
        let x = vec![0.0, 0.5, -0.5, 0.99];
        let (y, rate) = super::decode(&super::encode(&x, 16_000)).unwrap();
        assert_eq!(rate, 16_000);
        for (a, b) in x.iter().zip(&y) {
            assert!((a - b).abs() < 1e-3);
        }
    }
}
