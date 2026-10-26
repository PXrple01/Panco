//! Vergleichs-Engine: Windows-Spracherkennung (WinRT, offline mit installiertem Sprachpaket).
//!
//! Wichtig: Freies Diktat ("Topic"-Grammatik) schickt Audio an Microsoft-Server – das ist
//! verboten. Darum nur eine Phrasenliste (alle Befehle × Spiele/Programme), die offline läuft.
//! Nachteil: versteht nur, was in der Liste steht. Die Erkennung nutzt ihr eigenes Mikrofon.

use crate::stt::Transcript;
use std::time::{Duration, Instant};
use windows::core::HSTRING;
use windows::Foundation::TimeSpan;
use windows::Globalization::Language;
use windows::Media::SpeechRecognition::{
    SpeechRecognitionListConstraint, SpeechRecognitionResultStatus, SpeechRecognizer,
};
use windows_collections::IIterable;

pub fn recognize_once(phrases: &[String]) -> Result<Transcript, String> {
    let t = Instant::now();
    let err = |e: windows::core::Error| format!("Windows-Spracherkennung: {}", e.message());
    let lang = Language::CreateLanguage(&HSTRING::from("de-DE")).map_err(err)?;
    let rec = SpeechRecognizer::Create(&lang).map_err(err)?;
    let list: Vec<HSTRING> = phrases.iter().map(HSTRING::from).collect();
    let iterable: IIterable<HSTRING> = list.into();
    let constraint = SpeechRecognitionListConstraint::Create(&iterable).map_err(err)?;
    rec.Constraints().map_err(err)?.Append(&constraint).map_err(err)?;
    let compiled = rec.CompileConstraintsAsync().map_err(err)?.join().map_err(err)?;
    if compiled.Status().map_err(err)? != SpeechRecognitionResultStatus::Success {
        return Err("Deutsches Sprachpaket fehlt (Windows-Einstellungen → Sprache)".into());
    }
    let timeouts = rec.Timeouts().map_err(err)?;
    let span = |d: Duration| TimeSpan { Duration: (d.as_millis() * 10_000) as i64 };
    let _ = timeouts.SetInitialSilenceTimeout(span(Duration::from_secs(4)));
    let _ = timeouts.SetEndSilenceTimeout(span(Duration::from_millis(700)));
    let load_ms = t.elapsed().as_millis() as u64;

    let t = Instant::now();
    let result = rec.RecognizeAsync().map_err(err)?.join().map_err(err)?;
    let text = if result.Status().map_err(err)? == SpeechRecognitionResultStatus::Success {
        result.Text().map_err(err)?.to_string()
    } else {
        String::new()
    };
    let stt_ms = t.elapsed().as_millis() as u64;
    Ok(Transcript { text, engine: "windows", audio_ms: stt_ms, load_ms, stt_ms })
}
