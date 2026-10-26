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
        }
    }
}
