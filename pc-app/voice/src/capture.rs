//! Mikrofon über cpal (Windows: WASAPI, geteilter Modus). Der Audio-Callback mischt nur auf
//! Mono und reicht die Samples weiter – alle Verarbeitung passiert im Pipeline-Thread.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

pub struct Mic {
    _stream: cpal::Stream,
    pub rate: u32,
}

/// Öffnet das Standard-Mikrofon. `sink` bekommt Mono-Blöcke in der Geräterate.
pub fn open(sink: impl Fn(Vec<f32>) + Send + 'static) -> Result<Mic, String> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("Kein Mikrofon gefunden")?;
    let supported = device.default_input_config().map_err(|e| format!("Mikrofon nicht nutzbar: {e}"))?;
    let rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    let mut config = supported.config();
    // Größere Puffer = seltener aufwachen. ~40 ms reichen für Wake-Word und Befehle locker.
    if let cpal::SupportedBufferSize::Range { min, max } = *supported.buffer_size() {
        config.buffer_size = cpal::BufferSize::Fixed((rate / 25).clamp(min, max));
    }
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, channels, sink),
        SampleFormat::I16 => build::<i16>(&device, &config, channels, sink),
        SampleFormat::U16 => build::<u16>(&device, &config, channels, sink),
        SampleFormat::I32 => build::<i32>(&device, &config, channels, sink),
        other => return Err(format!("Mikrofonformat {other:?} nicht unterstützt")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Mic { _stream: stream, rate })
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    sink: impl Fn(Vec<f32>) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            *config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mono: Vec<f32> = data
                    .chunks(channels)
                    .map(|frame| frame.iter().map(|s| f32::from_sample_(*s)).sum::<f32>() / channels as f32)
                    .collect();
                sink(mono);
            },
            |e| eprintln!("[panco] Mikrofonfehler: {e}"),
            None,
        )
        .map_err(|e| format!("Mikrofon konnte nicht geöffnet werden: {e}"))
}
