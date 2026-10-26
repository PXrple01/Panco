//! Natürliche Stimme: Piper (neuronale Sprachausgabe, läuft komplett lokal).
//!
//! - Laufzeit (piper.exe + ONNX Runtime, ~22 MB) und Stimmen (20–63 MB) werden nur auf Klick
//!   geladen und per SHA-256 geprüft – wie die Whisper-Modelle.
//! - piper läuft als Kindprozess nur, solange gesprochen wird, und endet nach der Haltezeit
//!   (Standard 60 s) wieder. Im Leerlauf kostet die Stimme also nichts.
//! - Tonhöhe ohne Rechenaufwand: Die WAV wird mit höherer Abtastrate abgespielt (heller und
//!   schneller), piper spricht dafür entsprechend langsamer (`--length_scale`).
//! - Die Sätze, die piper erzeugt, liegen nur Millisekunden in einem privaten Temp-Ordner und
//!   werden sofort nach dem Lesen gelöscht. Mikrofon-Audio hat damit nichts zu tun.

use crate::model::fetch;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct NaturalVoice {
    pub id: &'static str,
    pub label: &'static str,
    pub female: bool,
    #[serde(skip)]
    pub path: &'static str,
    pub size: u64,
    #[serde(skip)]
    pub sha256: &'static str,
    #[serde(skip)]
    pub json_size: u64,
    #[serde(skip)]
    pub json_sha256: &'static str,
}

const VOICE_URL: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main/de/de_DE/";

/// Deutsche Piper-Stimmen (Prüfsummen selbst nachgerechnet, Stand piper-voices 1.0).
pub const VOICES: [NaturalVoice; 4] = [
    NaturalVoice {
        id: "kerstin",
        label: "Kerstin – weiblich, warm (63 MB)",
        female: true,
        path: "kerstin/low/de_DE-kerstin-low",
        size: 63_104_526,
        sha256: "d352a7641892cebf2903859af94e9ba81a141110215fe3943bcda7f7da401b7a",
        json_size: 4158,
        json_sha256: "56e708556b7b9b7a53c4f8957e021421e69f11a600962bba554cffbe72cf2d47",
    },
    NaturalVoice {
        id: "ramona",
        label: "Ramona – weiblich, klar (63 MB)",
        female: true,
        path: "ramona/low/de_DE-ramona-low",
        size: 63_104_526,
        sha256: "a1068592532aad8c49bf3d1a01b8749c11d7dba726689ee28ad64e7b627ae4c5",
        json_size: 4157,
        json_sha256: "64906ab7a050ac685fa3dcea044baa798f67ed77be164f9c668a69298c6487d2",
    },
    NaturalVoice {
        id: "eva",
        label: "Eva – weiblich, hell, sehr klein (21 MB)",
        female: true,
        path: "eva_k/x_low/de_DE-eva_k-x_low",
        size: 20_628_813,
        sha256: "e88cf290fbfb768bf111330d2e8a46e376b0d85e3423a28bfebbc863a260dad8",
        json_size: 4158,
        json_sha256: "ef14b3dcb279ab4b18422a7a132877bee7a148821bd91152fb7ae9c4b3d79625",
    },
    NaturalVoice {
        id: "thorsten",
        label: "Thorsten – männlich, am natürlichsten (63 MB)",
        female: false,
        path: "thorsten/medium/de_DE-thorsten-medium",
        size: 63_201_294,
        sha256: "7e64762d8e5118bb578f2eea6207e1a35a8e0c30595010b666f983fc87bb7819",
        json_size: 4819,
        json_sha256: "974adee790533adb273a1ac88f49027d2a1b8f0f2cf4905954a4791e79264e85",
    },
];

struct Runtime {
    url: &'static str,
    size: u64,
    sha256: &'static str,
}

#[cfg(windows)]
const RUNTIME: Runtime = Runtime {
    url: "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_windows_amd64.zip",
    size: 22_477_236,
    sha256: "f3c58906402b24f3a96d92145f58acba6d86c9b5db896d207f78dc80811efcea",
};
/// Nur für die Entwicklung unter Linux.
#[cfg(not(windows))]
const RUNTIME: Runtime = Runtime {
    url: "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_linux_x86_64.tar.gz",
    size: 26_460_462,
    sha256: "a50cb45f355b7af1f6d758c1b360717877ba0a398cc8cbe6d2a7a3a26e225992",
};

pub fn voice(id: &str) -> Option<&'static NaturalVoice> {
    VOICES.iter().find(|v| v.id == id)
}

fn file_name(v: &NaturalVoice) -> &'static str {
    v.path.rsplit('/').next().unwrap_or(v.path)
}

pub fn model_path(dir: &Path, id: &str) -> Option<PathBuf> {
    voice(id).map(|v| dir.join(format!("{}.onnx", file_name(v))))
}

pub fn exe_path(dir: &Path) -> PathBuf {
    dir.join("piper").join(if cfg!(windows) { "piper.exe" } else { "piper" })
}

pub fn installed(dir: &Path, id: &str) -> bool {
    let Some(model) = model_path(dir, id) else { return false };
    let mut json = model.clone().into_os_string();
    json.push(".json");
    exe_path(dir).is_file() && model.is_file() && Path::new(&json).is_file()
}

/// Laufzeit (falls noch nicht da) und Stimme laden. `progress(geladen, gesamt)` über alles.
pub fn install(dir: &Path, id: &str, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    let v = voice(id).ok_or("unbekannte Stimme")?;
    let need_runtime = !exe_path(dir).is_file();
    let total = v.size + v.json_size + if need_runtime { RUNTIME.size } else { 0 };
    let mut offset = 0;
    if need_runtime {
        let archive = dir.join(if cfg!(windows) { "piper-laufzeit.zip" } else { "piper-laufzeit.tar.gz" });
        fetch(RUNTIME.url, RUNTIME.size, RUNTIME.sha256, &archive, |d, _| progress(d, total))?;
        let result = extract(&archive, dir);
        let _ = std::fs::remove_file(&archive);
        result?;
        offset = RUNTIME.size;
    }
    let model = model_path(dir, id).ok_or("unbekannte Stimme")?;
    let mut json = model.clone().into_os_string();
    json.push(".json");
    fetch(&format!("{VOICE_URL}{}.onnx.json", v.path), v.json_size, v.json_sha256, Path::new(&json), |_, _| {})?;
    fetch(&format!("{VOICE_URL}{}.onnx", v.path), v.size, v.sha256, &model, |d, _| progress(offset + d, total))?;
    Ok(())
}

/// Stimme löschen; die Laufzeit nur, wenn keine Stimme mehr übrig ist.
pub fn remove(dir: &Path, id: &str) -> Result<(), String> {
    let model = model_path(dir, id).ok_or("unbekannte Stimme")?;
    let mut json = model.clone().into_os_string();
    json.push(".json");
    let _ = std::fs::remove_file(&model);
    let _ = std::fs::remove_file(&json);
    if !VOICES.iter().any(|v| installed(dir, v.id)) {
        let _ = std::fs::remove_dir_all(dir.join("piper"));
    }
    Ok(())
}

/// Nur Dateien unterhalb von `piper/` – Einträge mit ".." oder absolutem Pfad werden abgelehnt.
#[cfg(windows)]
fn extract(archive: &Path, dir: &Path) -> Result<(), String> {
    extract_zip(archive, dir)
}

#[cfg(not(windows))]
fn extract(archive: &Path, dir: &Path) -> Result<(), String> {
    // Entwicklungsumgebung: das Linux-Archiv ist ein tar.gz
    let ok = Command::new("tar")
        .arg("xzf")
        .arg(archive)
        .arg("-C")
        .arg(dir)
        .arg("--no-same-owner")
        .arg("piper")
        .status()
        .map_err(|e| e.to_string())?;
    if ok.success() {
        Ok(())
    } else {
        Err("Archiv konnte nicht entpackt werden".into())
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn extract_zip(archive: &Path, dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("Archiv beschädigt: {e}"))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = entry.enclosed_name() else { return Err("ungültiger Pfad im Archiv".into()) };
        if !name.starts_with("piper") {
            continue;
        }
        let target = dir.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Abtastrate der Stimme aus ihrer `.onnx.json`.
pub fn sample_rate(model: &Path) -> u32 {
    let mut json = model.to_path_buf().into_os_string();
    json.push(".json");
    std::fs::read(&json)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["audio"]["sample_rate"].as_u64())
        .map_or(22_050, |r| r as u32)
}

/// Text in Sätze teilen, damit der erste Satz schon spielt, während der nächste entsteht.
/// Sehr kurze Stücke ("Ok.") werden mit dem folgenden zusammengelegt.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        cur.push(if c == '\n' || c == '\r' { ' ' } else { c });
        let end = matches!(c, '.' | '!' | '?' | '…' | '\n') && chars.get(i + 1).is_none_or(|n| n.is_whitespace());
        if end && cur.trim().chars().filter(|c| c.is_alphanumeric()).count() >= 12 {
            out.push(cur.trim().to_string());
            cur.clear();
        }
    }
    if cur.trim().chars().any(|c| c.is_alphanumeric()) {
        out.push(cur.trim().to_string());
    }
    out
}

/// Ein laufender piper-Prozess mit geladener Stimme.
pub struct Piper {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    out_dir: PathBuf,
    rate: u32,
    pitch: f32,
}

impl Piper {
    /// `pitch`/`tempo`: 0.5 … 2.0 (1.0 = Original).
    pub fn start(exe: &Path, model: &Path, pitch: f32, tempo: f32) -> Result<Self, String> {
        let pitch = pitch.clamp(0.7, 1.6);
        let tempo = tempo.clamp(0.5, 2.0);
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let out_dir = std::env::temp_dir().join(format!(
            "panco-stimme-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(exe);
        cmd.arg("--model")
            .arg(model)
            .arg("--output_dir")
            .arg(&out_dir)
            // Höhere Abspielrate macht schneller → piper spricht um denselben Faktor langsamer.
            .arg("--length_scale")
            .arg(format!("{:.3}", pitch / tempo))
            .arg("--sentence_silence")
            .arg("0.1")
            .arg("--quiet")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(dir) = exe.parent() {
            cmd.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().map_err(|e| format!("Stimme startet nicht: {e}"))?;
        let stdin = child.stdin.take().ok_or("kein Eingang")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("kein Ausgang")?);
        Ok(Self { child, stdin, stdout, out_dir, rate: sample_rate(model), pitch })
    }

    /// Einen Satz erzeugen → WAV im Speicher (bereits auf die Tonhöhe umgestellt).
    pub fn synth(&mut self, sentence: &str) -> Result<Vec<u8>, String> {
        let line: String = sentence.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        if !line.chars().any(|c| c.is_alphanumeric()) {
            return Err("nichts zu sprechen".into());
        }
        writeln!(self.stdin, "{}", line.trim()).and_then(|_| self.stdin.flush()).map_err(|e| e.to_string())?;
        let mut path = String::new();
        if self.stdout.read_line(&mut path).map_err(|e| e.to_string())? == 0 {
            return Err("Stimme beendet".into());
        }
        let path = PathBuf::from(path.trim());
        // Nur Dateien aus dem eigenen Ordner lesen und gleich wieder löschen.
        if path.parent() != Some(self.out_dir.as_path()) {
            return Err("unerwartete Ausgabe der Stimme".into());
        }
        let wav = std::fs::read(&path).map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&path);
        retune(&wav?, (self.rate as f32 * self.pitch) as u32).ok_or_else(|| "unbekanntes Audioformat".into())
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Piper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.out_dir);
    }
}

/// 16-Bit-Mono-WAV mit neuer Abspielrate (Datenblock bleibt unverändert).
pub fn retune(wav: &[u8], rate: u32) -> Option<Vec<u8>> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12;
    let (mut channels, mut bits) = (0u16, 0u16);
    while pos + 8 <= wav.len() {
        let id = &wav[pos..pos + 4];
        let len = u32::from_le_bytes(wav[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = wav.get(pos + 8..(pos + 8 + len).min(wav.len()))?;
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            if bits != 16 || channels != 1 {
                return None;
            }
            let mut out = Vec::with_capacity(44 + body.len());
            out.extend_from_slice(b"RIFF");
            out.extend_from_slice(&(36 + body.len() as u32).to_le_bytes());
            out.extend_from_slice(b"WAVEfmt ");
            out.extend_from_slice(&16u32.to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&rate.to_le_bytes());
            out.extend_from_slice(&(rate * 2).to_le_bytes());
            out.extend_from_slice(&2u16.to_le_bytes());
            out.extend_from_slice(&16u16.to_le_bytes());
            out.extend_from_slice(b"data");
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(body);
            return Some(out);
        }
        pos += 8 + len + (len & 1);
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn split_into_sentences() {
        assert_eq!(
            sentences("Ok. Ich habe Discord gestartet. Soll ich auch Spotify öffnen?"),
            vec!["Ok. Ich habe Discord gestartet.", "Soll ich auch Spotify öffnen?"]
        );
        assert_eq!(sentences("Version 2.5 ist da"), vec!["Version 2.5 ist da"]);
        assert!(sentences(" … ").is_empty());
    }

    #[test]
    fn retune_keeps_samples() {
        let wav = crate::wav::encode(&[0.1, -0.2, 0.3], 16_000);
        let out = retune(&wav, 18_000).unwrap();
        let (samples, rate) = crate::wav::decode(&out).unwrap();
        assert_eq!(rate, 18_000);
        assert_eq!(samples.len(), 3);
        assert!(retune(b"kaputt", 1).is_none());
    }

    #[test]
    fn catalog_paths() {
        let dir = Path::new("/x");
        assert_eq!(model_path(dir, "kerstin").unwrap(), Path::new("/x/de_DE-kerstin-low.onnx"));
        assert!(model_path(dir, "nope").is_none());
        assert!(!installed(dir, "kerstin"));
    }

    /// Zip-Archive: nur `piper/…`, kein Ausbruch aus dem Zielordner.
    #[test]
    fn zip_extraction_is_contained() {
        let dir = std::env::temp_dir().join(format!("panco-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("a.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("piper/piper.exe", o).unwrap();
            z.write_all(b"exe").unwrap();
            z.start_file("anderes/datei.txt", o).unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        let out = dir.join("out");
        extract_zip(&archive, &out).unwrap();
        assert_eq!(std::fs::read(out.join("piper/piper.exe")).unwrap(), b"exe");
        assert!(!out.join("anderes").exists());

        let evil = dir.join("b.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&evil).unwrap());
            z.start_file("../../böse.txt", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        assert!(extract_zip(&evil, &out).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Nachgebauter piper (Shell-Skript): liest Zeilen, schreibt pro Zeile eine WAV mit 0,1 s
    /// in `--output_dir` und gibt den Pfad aus. Liefert (Ordner, piper, Stimme).
    #[cfg(unix)]
    pub(crate) fn fake_piper(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("panco-fakepiper-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sample = dir.join("probe.wav");
        std::fs::write(&sample, crate::wav::encode(&[0.0; 1600], 16_000)).unwrap();
        let model = dir.join("stimme.onnx");
        std::fs::write(&model, b"").unwrap();
        std::fs::write(dir.join("stimme.onnx.json"), br#"{"audio":{"sample_rate":16000}}"#).unwrap();
        // Liest Zeilen, schreibt pro Zeile eine WAV in --output_dir und gibt den Pfad aus.
        let exe = dir.join("piper");
        std::fs::write(
            &exe,
            format!(
                "#!/bin/sh\nout=\"$4\"\nn=0\nwhile read line; do n=$((n+1)); cp '{}' \"$out/$n.wav\"; echo \"$out/$n.wav\"; done\n",
                sample.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, exe, model)
    }

    /// Protokoll: Satz rein, WAV-Pfad raus, Datei sofort wieder weg.
    #[cfg(unix)]
    #[test]
    fn talks_to_piper_process() {
        let (dir, exe, model) = fake_piper("proto");
        let mut p = Piper::start(&exe, &model, 1.25, 1.0).unwrap();
        let out_dir = p.out_dir.clone();
        let wav = p.synth("Hallo, ich bin Panco.").unwrap();
        let (samples, rate) = crate::wav::decode(&wav).unwrap();
        assert_eq!((samples.len(), rate), (1600, 20_000));
        assert!(p.synth("Zweiter Satz").is_ok());
        assert!(std::fs::read_dir(&out_dir).unwrap().next().is_none(), "Sätze werden sofort gelöscht");
        assert!(p.alive());
        drop(p);
        assert!(!out_dir.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
