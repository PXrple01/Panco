# /shared

Wird von PC-Overlay und PWA gemeinsam genutzt – nur hier ändern.

| Pfad | Inhalt |
|---|---|
| `tokens.css` | Farben, Radien, Schrift, Easing als CSS-Variablen (Theme) |
| `fonts/` | Inter (variabel, Latin inkl. Umlaute), lokal eingebettet, OFL-Lizenz |
| `avatar/panco.svg` | Panco aus Einzelteilen (`p-ear`, `p-eye`, `p-pupil`, `p-arm`, `p-tail`, `p-crystal`, `p-cheek`, …) |
| `avatar/panco.css` | Animationen (nur `transform`/`opacity`) |
| `avatar/panco.ts` | Mini-Controller: zufälliges Blinzeln, Pause bei unsichtbarer Seite |
| `schema/actions.json` | Aktions-Whitelist mit Stufen, Parametern, Remote-Freigabe (auch Quelle für Claude-Tools) |
| `schema/messages.schema.json` | Nachrichtenformat PC ↔ Overlay/PWA (JSON Schema) |
| `schema/messages.ts` | TypeScript-Spiegel des Nachrichtenformats |
| `texts/de.json` | Alle Panco-Sätze |

Vorschau: `npm run -w tools avatar:dev` · Messung: `npm run -w tools avatar:measure`
