# Panco PC-App (Windows 11)

Tauri 2 (Rust) + Vanilla TypeScript. Kein Electron.

```
pc-app/
  core/       panco-core: Parser, Aktions-Whitelist mit Stufen, Engine, Steam, Protokoll (plattformunabhängig, getestet)
  voice/      panco-voice: Mikrofon, VAD, Wake-Word (rustpotter), Whisper (whisper.cpp), Windows-TTS
  src-tauri/  App: Tray, Hotkeys, Overlay/Einstellungsfenster auf Abruf, IPC
  ui/         Overlay (index.html) und Einstellungen & Diagnose (settings.html)
```

## Unter Windows bauen

Einmalig installieren (PowerShell als normaler Benutzer):

```powershell
winget install Rustlang.Rustup OpenJS.NodeJS.LTS Kitware.CMake LLVM.LLVM
winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive"
```

- **CMake + MSVC** bauen whisper.cpp, **LLVM** wird für die Rust-Bindings (bindgen) gebraucht.
- WebView2 ist in Windows 11 bereits enthalten.

Dann im Repo-Ordner:

```powershell
npm install
cd pc-app
npm run dev      # Entwicklung (Overlay mit Live-Reload)
npm run build    # Installer: target\release\bundle\nsis\Panco_0.1.0_x64-setup.exe
```

Tests (laufen auch unter Linux/macOS): `cargo test` im Ordner `pc-app`.

## Erste Schritte nach dem Start

1. Tray-Icon (Panda) → **Einstellungen & Diagnose**
2. Whisper-Modell **Base** herunterladen (57 MB, einmalig, mit Prüfsumme)
3. **Wake-Word einsprechen**: 6× „Panco“ sagen
4. Fertig: „Panco, öffne Deadlock“ – oder **Strg+Alt+Leertaste** halten und sprechen

| Taste | Funktion |
|---|---|
| Strg + Leertaste | Overlay mit Befehlszeile |
| Strg + Alt + Leertaste (halten) | Push-to-Talk |
| Strg + Alt + M | Mikrofon stumm |
| Strg + Alt + Ende | Not-Aus |

Alle Daten liegen in `%APPDATA%\Panco` (config.json, logs, models, wakeword).

## Messen

```powershell
powershell -ExecutionPolicy Bypass -File ..\tools\measure.ps1 -Voice
```

## Vorschau-Build aus Linux

`tools/cross/build-windows.sh` baut eine `.exe` mit MinGW (für Tests ohne Windows-Toolchain).
Für echte Releases unter Windows mit MSVC bauen.
