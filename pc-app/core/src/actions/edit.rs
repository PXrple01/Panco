//! file.edit – Textdatei ändern (Stufe 2): Vorschau als Diff, Bestätigung am PC, vorher eine
//! Kopie in den Panco-Papierkorb. "Rückgängig" holt die alte Fassung zurück.

use super::read::{file_label, read_text, user_path};
use super::*;
use crate::texts::t;
use serde::Deserialize;

pub struct FileEdit;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct P {
    path: String,
    find: Option<String>,
    replace: Option<String>,
    all: Option<bool>,
    content: Option<String>,
}

impl Action for FileEdit {
    fn id(&self) -> &'static str {
        "file.edit"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: P = super::params(params)?;
        let path = user_path(&p.path)?;
        let name = file_label(&path);
        let tier = ctx.places.write_tier(&path, Tier::Confirm).map_err(ActionError::NotFound)?;
        if path.to_string_lossy().ends_with(".panco") || is_active(&path) {
            return Err(ActionError::NotFound("Diese Datei bearbeite ich nicht.".into()));
        }
        let exists = path.exists();
        let old = if exists { read_text(&path)? } else { String::new() };
        let new = match (&p.find, &p.content) {
            (Some(find), None) => {
                if !exists {
                    return Err(ActionError::NotFound(format!("„{name}“ gibt es nicht.")));
                }
                let replace = p.replace.clone().unwrap_or_default();
                let n = old.matches(find.as_str()).count();
                if n == 0 {
                    return Err(ActionError::NotFound(t("file_edit_not_found", &[("name", &name)])));
                }
                if n > 1 && !p.all.unwrap_or(false) {
                    return Err(ActionError::NotFound(t(
                        "file_edit_ambiguous",
                        &[("name", &name), ("n", &n.to_string())],
                    )));
                }
                old.replace(find.as_str(), &replace)
            }
            (None, Some(content)) => content.clone(),
            _ => return Err(ActionError::InvalidParams("entweder find/replace oder content".into())),
        };
        if new == old && exists {
            return Err(ActionError::NotFound(t("file_edit_same", &[])));
        }
        let (diff, added, removed) = diff(&old, &new, &name);
        let summary = format!("+{added} −{removed} Zeilen");
        let what = t("file_edit_what", &[("name", &name), ("summary", &summary)]);
        let trash = crate::files::trash::Trash::new(ctx.places.trash.clone());
        Ok(Plan::ready(tier.max(Tier::Confirm), what, move |_| {
            // Vorher sichern (Original bleibt bis zum Schreiben unberührt).
            let undo = if exists {
                let entry = trash.snapshot(&path).map_err(|e| ActionError::Failed(e.to_string()))?;
                Some(crate::undo::UndoOp::Restore { trash_id: entry.id, name: name.clone() })
            } else {
                None
            };
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| ActionError::Failed(e.to_string()))?;
            }
            let tmp = path.with_extension("panco-tmp");
            std::fs::write(&tmp, new.as_bytes()).map_err(|e| ActionError::Failed(e.to_string()))?;
            std::fs::rename(&tmp, &path).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                ActionError::Failed(e.to_string())
            })?;
            Ok(match undo {
                Some(op) => Done::ok(t("file_edit_done", &[("name", &name)])).undo(op),
                None => Done::ok(t("file_edit_created", &[("name", &name)]))
                    .undo(crate::undo::UndoOp::Trash { path: path.clone() }),
            })
        })
        .with_detail(diff))
    }
}

fn is_active(p: &std::path::Path) -> bool {
    crate::files::is_active_file(p)
}

/// Zeilen-Diff mit 2 Zeilen Kontext (höchstens 400 Zeilen Ausgabe). Liefert (Text, +, −).
pub fn diff(old: &str, new: &str, name: &str) -> (String, usize, usize) {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // gemeinsamer Anfang/Ende abschneiden, dann LCS auf dem Rest (wenn klein genug)
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        suf += 1;
    }
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mut ops: Vec<(char, &str)> = Vec::new();
    if am.len() * bm.len() <= 4_000_000 {
        let (n, m) = (am.len(), bm.len());
        let mut l = vec![vec![0u32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                l[i][j] = if am[i] == bm[j] { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                ops.push((' ', am[i]));
                i += 1;
                j += 1;
            } else if j < m && (i == n || l[i][j + 1] >= l[i + 1][j]) {
                ops.push(('+', bm[j]));
                j += 1;
            } else {
                ops.push(('-', am[i]));
                i += 1;
            }
        }
    } else {
        ops.extend(am.iter().map(|l| ('-', *l)));
        ops.extend(bm.iter().map(|l| ('+', *l)));
    }
    let added = ops.iter().filter(|o| o.0 == '+').count();
    let removed = ops.iter().filter(|o| o.0 == '-').count();
    // Ausgabe: Kontext vor/nach den Änderungen
    let mut out = vec![format!("--- {name} (vorher)"), format!("+++ {name} (nachher)")];
    let ctx_before: Vec<&str> = a[pre.saturating_sub(2)..pre].to_vec();
    out.push(format!("@@ ab Zeile {} @@", pre.saturating_sub(2) + 1));
    out.extend(ctx_before.iter().map(|l| format!("  {l}")));
    let mut last_change = None;
    for (k, (op, _)) in ops.iter().enumerate() {
        if *op != ' ' {
            last_change = Some(k);
        }
    }
    let mut skipping = false;
    for (k, (op, line)) in ops.iter().enumerate() {
        let near = ops[k.saturating_sub(2)..(k + 3).min(ops.len())].iter().any(|o| o.0 != ' ');
        if *op == ' ' && !near {
            if !skipping {
                out.push("  …".into());
                skipping = true;
            }
            continue;
        }
        skipping = false;
        out.push(format!("{op} {line}"));
        if out.len() > 400 {
            out.push(format!("… ({} weitere Zeilen)", ops.len() - k));
            break;
        }
    }
    if last_change.is_some() {
        out.extend(b[b.len() - suf..].iter().take(2).map(|l| format!("  {l}")));
    }
    (out.join("\n"), added, removed)
}

#[cfg(test)]
mod tests {
    #[test]
    fn small_diff() {
        let (d, add, rem) = super::diff("a\nb\nc\nd\ne\nf\ng\n", "a\nb\nc\nD\ne\nf\ng\n", "x.txt");
        assert_eq!((add, rem), (1, 1));
        assert!(d.contains("- d") && d.contains("+ D"), "{d}");
        assert!(d.contains("  c") && d.contains("  e"), "Kontext: {d}");
        let (_, add, rem) = super::diff("", "neu\n", "y.txt");
        assert_eq!((add, rem), (1, 0));
    }
}
