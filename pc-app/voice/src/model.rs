//! Whisper-Modelle: Katalog, Pfade, Download mit SHA-256-Prüfung.
//! Download passiert nur auf Klick in den Einstellungen – nie automatisch im Hintergrund.

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct ModelInfo {
    pub id: &'static str,
    pub file: &'static str,
    pub label: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

/// Quantisierte Modelle (q5_1): klein im RAM, Deutsch ab "base" brauchbar, "small" deutlich besser.
pub const MODELS: [ModelInfo; 3] = [
    ModelInfo {
        id: "tiny",
        file: "ggml-tiny-q5_1.bin",
        label: "Tiny (31 MB) – sehr schnell, Deutsch schwach",
        size: 32_152_673,
        sha256: "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7",
    },
    ModelInfo {
        id: "base",
        file: "ggml-base-q5_1.bin",
        label: "Base (57 MB) – Standard, schnell",
        size: 59_707_625,
        sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
    },
    ModelInfo {
        id: "small",
        file: "ggml-small-q5_1.bin",
        label: "Small (181 MB) – genauer, langsamer",
        size: 190_085_487,
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
    },
];

pub fn info(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

pub fn path(dir: &Path, id: &str) -> PathBuf {
    dir.join(info(id).map_or("ggml-base-q5_1.bin", |m| m.file))
}

/// Lädt ein Modell herunter (`.part`-Datei, Prüfsumme, dann umbenennen).
/// `progress(geladen, gesamt)` wird ca. alle 256 KB aufgerufen.
pub fn download(id: &str, dir: &Path, progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    let m = info(id).ok_or("unbekanntes Modell")?;
    let target = dir.join(m.file);
    fetch(&format!("{BASE_URL}{}", m.file), m.size, m.sha256, &target, progress)?;
    Ok(target)
}

/// Eine Datei herunterladen und nur bei passender SHA-256-Prüfsumme behalten.
pub fn fetch(
    url: &str,
    size: u64,
    sha256: &str,
    target: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut part = target.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .build()
        .into();
    let mut resp = agent.get(url).call().map_err(|e| format!("Download fehlgeschlagen: {e}"))?;
    let mut reader = resp.body_mut().with_config().limit(size + 1024).reader();
    let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("Download abgebrochen: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress(done, size);
    }
    drop(file);
    let hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if hash != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err("Prüfsumme stimmt nicht – Datei verworfen".into());
    }
    std::fs::rename(&part, target).map_err(|e| e.to_string())
}
