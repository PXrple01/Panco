//! Spracherkennung ohne Wake-Word bewerten (gleiche Einstellungen wie in Panco).
//! cargo run -p panco-voice --release --example stt_eval -- <modell> "<prompt>" a.wav b.wav …
use panco_voice::resample::Resampler;
use panco_voice::stt::WhisperStt;
use panco_voice::wav;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut stt = WhisperStt::new(args[1].clone().into());
    // PANCO_STT_QUALITY=fast|balanced|precise
    stt.set_quality(match std::env::var("PANCO_STT_QUALITY").as_deref() {
        Ok("fast") => panco_voice::stt::Quality::Fast,
        Ok("precise") => panco_voice::stt::Quality::Precise,
        _ => panco_voice::stt::Quality::Balanced,
    });
    let prompt = &args[2];
    for f in &args[3..] {
        let (s, rate) = wav::decode(&std::fs::read(f).unwrap()).unwrap();
        let mut pcm = Vec::new();
        Resampler::new(rate).process(&s, &mut pcm);
        let t = stt.transcribe(&pcm, prompt).unwrap();
        let name = std::path::Path::new(f).file_stem().unwrap().to_string_lossy().to_string();
        println!("{name:<24} {:>5} ms  {}", t.stt_ms, t.text);
    }
}
