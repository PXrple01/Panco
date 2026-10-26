//! Rückgängig: Aktionen hinterlegen im Protokoll, wie sie umgekehrt werden.
//! Neue umkehrbare Aktion = neue Variante hier + `describe` + `apply`.

use crate::files::trash::Trash;
use crate::files::{free_name, move_path};
use crate::platform::Platform;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum UndoOp {
    /// Aus dem Panco-Papierkorb zurückholen (gelöscht oder Sicherungskopie).
    Restore {
        trash_id: String,
        name: String,
    },
    /// Verschieben/Umbenennen umkehren: `from` (jetzt) → `to` (vorher).
    MoveBack {
        from: PathBuf,
        to: PathBuf,
    },
    /// Neu Angelegtes in den Papierkorb legen.
    Trash {
        path: PathBuf,
    },
    /// Angehängten Text wieder entfernen (Datei auf alte Länge kürzen).
    Truncate {
        path: PathBuf,
        len: u64,
    },
    Volume {
        level: u8,
    },
    Mute {
        muted: bool,
    },
    /// Merksatz, Regel oder eigenen Befehl wieder entfernen.
    MemoryRemove {
        file: PathBuf,
        id: String,
    },
    /// Mehrere Schritte auf einmal (z. B. alle Sicherungen eines Skripts), rückwärts angewendet.
    Batch {
        ops: Vec<UndoOp>,
    },
    /// Lautstärke eines Programms zurücksetzen.
    AppVolume {
        process: String,
        level: u8,
        muted: bool,
    },
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}

impl UndoOp {
    /// Was "Rückgängig" tun wird – für Knopf und Rückfrage.
    pub fn describe(&self) -> String {
        match self {
            UndoOp::Restore { name, .. } => format!("{name} wiederherstellen"),
            UndoOp::MoveBack { from, to } => {
                if from.parent() == to.parent() {
                    format!("{} wieder in {} umbenennen", file_name(from), file_name(to))
                } else {
                    format!(
                        "{} zurück nach {} verschieben",
                        file_name(from),
                        to.parent().map(file_name).unwrap_or_default()
                    )
                }
            }
            UndoOp::Trash { path } => format!("{} wieder entfernen (Papierkorb)", file_name(path)),
            UndoOp::Truncate { path, .. } => format!("Ergänzung in {} entfernen", file_name(path)),
            UndoOp::Volume { level } => format!("Lautstärke zurück auf {level}"),
            UndoOp::Mute { muted } => (if *muted { "wieder stumm schalten" } else { "Ton wieder an" }).into(),
            UndoOp::MemoryRemove { .. } => "wieder vergessen".into(),
            UndoOp::AppVolume { process, level, .. } => {
                format!("{} zurück auf {level}", process.trim_end_matches(".exe"))
            }
            UndoOp::Batch { ops } => format!("{} Änderungen zurücknehmen", ops.len()),
        }
    }

    /// Führt die Umkehrung aus und liefert den Antwortsatz.
    pub fn apply(&self, pf: &dyn Platform, trash: &Trash) -> Result<String, String> {
        let io = |e: std::io::Error| e.to_string();
        match self {
            UndoOp::Restore { trash_id, name } => {
                let at = trash.restore(trash_id).map_err(io)?;
                Ok(format!("{name} ist wieder da ({}).", at.parent().map(file_name).unwrap_or_default()))
            }
            UndoOp::MoveBack { from, to } => {
                if !from.exists() {
                    return Err(format!("{} gibt es dort nicht mehr", file_name(from)));
                }
                let target = free_name(to);
                move_path(from, &target).map_err(io)?;
                Ok(format!("Rückgängig: {} ist zurück.", file_name(&target)))
            }
            UndoOp::Trash { path } => {
                trash.put(path).map_err(io)?;
                Ok(format!("{} liegt im Papierkorb.", file_name(path)))
            }
            UndoOp::Truncate { path, len } => {
                let f = std::fs::OpenOptions::new().write(true).open(path).map_err(io)?;
                f.set_len(*len).map_err(io)?;
                Ok("Ergänzung entfernt.".into())
            }
            UndoOp::Volume { level } => {
                pf.set_volume(*level).map_err(|e| e.to_string())?;
                Ok(format!("Lautstärke wieder {level}."))
            }
            UndoOp::Mute { muted } => {
                pf.set_muted(*muted).map_err(|e| e.to_string())?;
                Ok(if *muted { "Wieder stumm." } else { "Ton wieder an." }.into())
            }
            UndoOp::MemoryRemove { file, id } => {
                let dir = file.parent().ok_or("kein Datenordner")?;
                if file.file_name().is_some_and(|n| n == "befehle.json") {
                    let mut m = crate::memory::Macros::load(dir);
                    m.list.retain(|x| &x.id != id);
                    m.save(dir)?;
                } else {
                    let mut m = crate::memory::Memory::load(dir);
                    m.facts.retain(|x| &x.id != id);
                    m.rules.retain(|x| &x.id != id);
                    m.save(dir)?;
                }
                Ok("Wieder vergessen.".into())
            }
            UndoOp::Batch { ops } => {
                let mut errors = Vec::new();
                for op in ops.iter().rev() {
                    if let Err(e) = op.apply(pf, trash) {
                        errors.push(e);
                    }
                }
                if errors.is_empty() {
                    Ok(format!("{} Änderungen zurückgenommen.", ops.len()))
                } else {
                    Err(errors.join("; "))
                }
            }
            UndoOp::AppVolume { process, level, muted } => {
                pf.set_app_volume(process, Some(*level), Some(*muted)).map_err(|e| e.to_string())?;
                Ok(format!("{} wieder bei {level} %.", process.trim_end_matches(".exe")))
            }
        }
    }
}
