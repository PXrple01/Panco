//! Wake-Word-Datei aus WAV-Aufnahmen erzeugen (für Tests/Entwicklung; in der App gibt es dafür
//! "Wake-Word einsprechen"). cargo run -p panco-voice --example wake_train -- out.rpw a.wav b.wav …
use panco_voice::resample::Resampler;
use panco_voice::{wake, wav};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let samples: Vec<Vec<f32>> = args[2..]
        .iter()
        .map(|f| {
            let (s, rate) = wav::decode(&std::fs::read(f).expect("WAV lesbar")).expect("16-Bit-WAV");
            let mut out = Vec::new();
            Resampler::new(rate).process(&s, &mut out);
            wake::trim_silence(&out)
        })
        .collect();
    wake::train(&samples, 0.5, std::path::Path::new(&args[1])).expect("Training");
    println!("{} aus {} Aufnahmen geschrieben", args[1], samples.len());
}
