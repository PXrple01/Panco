//! Steam-Bibliothek lesen: `libraryfolders.vdf` → Bibliotheken → `appmanifest_*.acf` → Spiele.
//! Gestartet wird per `steam://rungameid/<id>` (siehe actions/launch.rs).

use crate::vdf;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Game {
    pub app_id: u32,
    pub name: String,
}

/// Steam-Werkzeuge, die keine Spiele sind.
fn is_tool(app_id: u32, name: &str) -> bool {
    const TOOL_IDS: [u32; 4] = [228980, 1070560, 1391110, 1628350]; // Redistributables, Linux-Runtimes
    let n = name.to_lowercase();
    TOOL_IDS.contains(&app_id)
        || n.starts_with("proton")
        || n.starts_with("steam linux runtime")
        || n.contains("redistributable")
        || n.contains("dedicated server")
}

/// Steam-Ordner finden: Konfig → Registry (Windows) → übliche Pfade.
pub fn find_root(configured: Option<&str>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = configured.filter(|p| !p.trim().is_empty()) {
        candidates.push(PathBuf::from(crate::config::expand_env(p)));
    }
    if let Some(p) = crate::platform::steam_path_from_registry() {
        candidates.push(p);
    }
    if cfg!(windows) {
        candidates.push(PathBuf::from(crate::config::expand_env("%ProgramFiles(x86)%\\Steam")));
        candidates.push(PathBuf::from("C:\\Program Files (x86)\\Steam"));
    } else if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".steam/steam"));
        candidates.push(home.join(".local/share/Steam"));
    }
    candidates.into_iter().find(|p| p.join("steamapps").is_dir())
}

/// Alle Bibliotheksordner (Hauptordner + weitere Laufwerke).
pub fn library_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = vec![root.to_path_buf()];
    let file = root.join("steamapps").join("libraryfolders.vdf");
    if let Some(doc) = std::fs::read_to_string(&file).ok().and_then(|s| vdf::parse(&s).ok()) {
        if let Some(folders) = doc.get("libraryfolders") {
            for (_, entry) in folders.entries() {
                if let Some(p) = entry.str("path") {
                    let p = PathBuf::from(p);
                    if !paths.iter().any(|x| same_path(x, &p)) {
                        paths.push(p);
                    }
                }
            }
        }
    }
    paths
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

/// Liest alle installierten Spiele, alphabetisch sortiert.
pub fn scan(root: &Path) -> Vec<Game> {
    let mut games = Vec::new();
    for lib in library_paths(root) {
        let Ok(dir) = std::fs::read_dir(lib.join("steamapps")) else { continue };
        for entry in dir.flatten() {
            let fname = entry.file_name();
            let fname = fname.to_string_lossy();
            if !(fname.starts_with("appmanifest_") && fname.ends_with(".acf")) {
                continue;
            }
            if let Some(game) = read_manifest(&entry.path()) {
                if !games.iter().any(|g: &Game| g.app_id == game.app_id) {
                    games.push(game);
                }
            }
        }
    }
    games.sort_by_key(|g| g.name.to_lowercase());
    games
}

fn read_manifest(path: &Path) -> Option<Game> {
    let doc = vdf::parse(&std::fs::read_to_string(path).ok()?).ok()?;
    let app = doc.get("AppState")?;
    let app_id: u32 = app.str("appid")?.parse().ok()?;
    let name = app.str("name")?.trim().to_string();
    (!name.is_empty() && !is_tool(app_id, &name)).then_some(Game { app_id, name })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!("panco-steam-{}", std::process::id()));
        let lib2 = root.join("lib2");
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::create_dir_all(lib2.join("steamapps")).unwrap();
        let vdf = format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
            root.display().to_string().replace('\\', "\\\\"),
            lib2.display().to_string().replace('\\', "\\\\")
        );
        std::fs::write(root.join("steamapps/libraryfolders.vdf"), vdf).unwrap();
        let acf = |id: u32, name: &str| format!("\"AppState\" {{ \"appid\" \"{id}\" \"name\" \"{name}\" }}");
        std::fs::write(root.join("steamapps/appmanifest_1422450.acf"), acf(1422450, "Deadlock")).unwrap();
        std::fs::write(
            root.join("steamapps/appmanifest_228980.acf"),
            acf(228980, "Steamworks Common Redistributables"),
        )
        .unwrap();
        std::fs::write(lib2.join("steamapps/appmanifest_730.acf"), acf(730, "Counter-Strike 2")).unwrap();
        std::fs::write(lib2.join("steamapps/notes.txt"), "kein manifest").unwrap();
        root
    }

    #[test]
    fn scans_all_libraries_and_skips_tools() {
        let root = fixture();
        let games = scan(&root);
        assert_eq!(
            games,
            vec![
                Game { app_id: 730, name: "Counter-Strike 2".into() },
                Game { app_id: 1422450, name: "Deadlock".into() }
            ]
        );
        assert_eq!(find_root(Some(root.to_str().unwrap())), Some(root.clone()));
        std::fs::remove_dir_all(root).ok();
    }
}
