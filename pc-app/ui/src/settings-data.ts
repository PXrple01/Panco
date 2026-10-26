// Reiter "Ordner & Backup", "Notizen", "Protokoll", "Papierkorb" im Einstellungsfenster.
// Daten werden erst beim Öffnen eines Reiters geladen (kein Dauer-Abfragen).
import { invoke } from "@tauri-apps/api/core";
import type { ServerMessage } from "../../../shared/schema/messages";
import { folderForm, type FileSettings } from "./folders";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text;
  return e;
}

function showError(msg: string | null) {
  const e = $("error");
  e.hidden = !msg;
  e.textContent = msg ?? "";
}

const date = (unix: number) =>
  new Date(unix * 1000).toLocaleString("de-DE", { day: "2-digit", month: "2-digit", year: "numeric", hour: "2-digit", minute: "2-digit" });
const size = (b: number) => (b < 1024 ? `${b} B` : b < 1048576 ? `${(b / 1024).toFixed(0)} KB` : `${(b / 1048576).toFixed(1)} MB`);

// ---------- Reiter ----------
const loaders: Record<string, () => void> = {};

function selectTab(name: string) {
  for (const t of document.querySelectorAll<HTMLButtonElement>(".tab")) t.setAttribute("aria-selected", String(t.dataset.tab === name));
  for (const p of document.querySelectorAll<HTMLElement>(".panel")) p.hidden = p.dataset.panel !== name;
  try {
    localStorage.setItem("panco-tab", name);
  } catch {}
  showError(null);
  loaders[name]?.();
  document.dispatchEvent(new CustomEvent("panco-tab", { detail: name }));
}
for (const t of document.querySelectorAll<HTMLButtonElement>(".tab")) t.addEventListener("click", () => selectTab(t.dataset.tab!));

// ---------- Ordner & Backup ----------
let readFolders: (() => FileSettings) | null = null;

interface BackupInfo {
  name: string;
  path: string;
  created: number;
}

async function loadBackups() {
  const list = $("backup-list");
  const backups = await invoke<BackupInfo[]>("backups");
  list.replaceChildren(
    ...(backups.length
      ? backups.map((b) => {
          const li = el("li");
          // "Panco-Backup-2026-10-26_08-02" → "Backup vom 26.10.2026, 08:02"
          const m = b.name.match(/(\d{4})-(\d{2})-(\d{2})_(\d{2})-(\d{2})/);
          const label = m ? `Backup vom ${m[3]}.${m[2]}.${m[1]}, ${m[4]}:${m[5]}` : b.name;
          const main = el("span", "row-main", label);
          main.append(el("span", "row-sub", b.path));
          li.append(main);
          return li;
        })
      : [el("li", "dim", "Noch kein Backup.")]),
  );
}

loaders.files = async () => {
  readFolders = folderForm($("folders"), await invoke<FileSettings>("files_settings"));
  await loadBackups();
};
$("files-save").addEventListener("click", async () => {
  if (!readFolders) return;
  try {
    await invoke("files_save", { files: readFolders() });
    $("files-status").textContent = "Gespeichert.";
    showError(null);
  } catch (e) {
    showError(String(e));
  }
});
$("backup-now").addEventListener("click", async () => {
  $("backup-status").textContent = "Sichere …";
  try {
    const report = await invoke<string[]>("backup_now");
    $("backup-status").textContent = report.join(" · ") || "Backup ist ausgeschaltet.";
    await loadBackups();
  } catch (e) {
    showError(String(e));
  }
});
$("backup-open").addEventListener("click", () => invoke("open_place", { which: "backup" }).catch((e) => showError(String(e))));

// ---------- Notizen ----------
interface NoteRow {
  title: string;
  sensitive: boolean;
  modified: number;
}
let currentNote: string | null = null;

async function loadNotes() {
  const notes = await invoke<NoteRow[]>("notes_list");
  const list = $("note-list");
  list.replaceChildren(
    ...(notes.length
      ? notes.map((n) => {
          const li = el("li", n.title === currentNote ? "is-active" : "");
          const main = el("span", "row-main", `${n.sensitive ? "🔒 " : ""}${n.title}`);
          main.append(el("span", "row-sub", date(n.modified)));
          li.append(main);
          li.addEventListener("click", () => openNote(n.title));
          return li;
        })
      : [el("li", "dim", "Noch keine Notizen. Sag z. B. „Panco, neue Notiz Einkauf“.")]),
  );
}

async function openNote(title: string) {
  try {
    const content = await invoke<string>("note_get", { title });
    currentNote = title;
    $("note-title").textContent = title;
    $("note-status").textContent = "";
    const ta = $<HTMLTextAreaElement>("note-text");
    ta.value = content;
    ta.disabled = false;
    $<HTMLButtonElement>("note-save").disabled = false;
    await loadNotes();
  } catch (e) {
    showError(String(e));
  }
}

loaders.notes = () => loadNotes();
$("note-save").addEventListener("click", async () => {
  if (!currentNote) return;
  try {
    await invoke("note_put", { title: currentNote, content: $<HTMLTextAreaElement>("note-text").value });
    $("note-status").textContent = "Gespeichert (alte Fassung liegt im Papierkorb).";
    await loadNotes();
  } catch (e) {
    showError(String(e));
  }
});
$("note-new").addEventListener("click", async () => {
  const title = $<HTMLInputElement>("note-new-title").value.trim();
  if (!title) return;
  const r = await invoke<ServerMessage>("note_new", { title, secret: $<HTMLInputElement>("note-new-secret").checked });
  if (r.type === "reply" && !r.ok) showError(r.text);
  $<HTMLInputElement>("note-new-title").value = "";
  await loadNotes();
  if (r.type === "reply" && r.ok) await openNote(title);
});
$("notes-open").addEventListener("click", () => invoke("open_place", { which: "notes" }).catch((e) => showError(String(e))));

// ---------- Protokoll ----------
interface LogRow {
  id: string;
  ts: string;
  source: string;
  text: string | null;
  action: string | null;
  title: string | null;
  outcome: string;
  message: string;
  undo: string | null;
  undone: boolean;
}
const SOURCE: Record<string, string> = { text: "getippt", voice: "Stimme", claude: "Claude", panco: "Panco" };
const OUTCOME: Record<string, string> = {
  done: "erledigt",
  confirmed: "bestätigt",
  pending: "Rückfrage",
  cancelled: "abgebrochen",
  expired: "abgelaufen",
  denied: "abgelehnt",
  failed: "fehlgeschlagen",
  not_understood: "nicht verstanden",
};

async function loadLog() {
  const rows = await invoke<LogRow[]>("log_entries", { limit: 150 });
  const list = $("log-list");
  list.replaceChildren(
    ...rows.map((r) => {
      const li = el("li", `${r.outcome === "failed" || r.outcome === "denied" ? "is-failed" : ""} ${r.undone ? "is-undone" : ""}`);
      const when = new Date(r.ts).toLocaleString("de-DE", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" });
      const src = r.source.startsWith("remote:") ? `📱 ${r.source.slice(7)}` : SOURCE[r.source] ?? r.source;
      const main = el("span", "row-main", r.message || r.title || r.text || r.action || r.outcome);
      main.append(
        el("span", "row-sub", [when, src, r.text && r.text !== r.message ? `„${r.text}“` : "", OUTCOME[r.outcome] ?? r.outcome, r.undone ? "rückgängig gemacht" : ""].filter(Boolean).join(" · ")),
      );
      li.append(main);
      if (r.undo) {
        const btn = el("button", "btn btn--small", "Rückgängig");
        btn.type = "button";
        btn.title = r.undo;
        btn.addEventListener("click", async () => {
          const res = await invoke<ServerMessage>("undo", { id: r.id });
          if (res.type === "reply" && !res.ok) showError(res.text);
          await loadLog();
        });
        li.append(btn);
      } else {
        li.append(el("span"));
      }
      return li;
    }),
  );
}
loaders.log = () => loadLog();

// ---------- Papierkorb ----------
interface TrashRow {
  id: string;
  name: string;
  original: string;
  deleted_at: number;
  size: number;
  is_dir: boolean;
  snapshot: boolean;
  days_left: number;
}

async function loadTrash() {
  const rows = await invoke<TrashRow[]>("trash_entries");
  const files = await invoke<FileSettings>("files_settings");
  $("trash-hint").textContent = `Einträge bleiben ${files.trash_days} Tage, dann entfernt Panco sie endgültig. Ordner: ${files.trash_dir}`;
  const list = $("trash-list");
  list.replaceChildren(
    ...(rows.length
      ? rows.map((r) => {
          const li = el("li");
          const main = el("span", "row-main", `${r.is_dir ? "📁 " : ""}${r.name}${r.snapshot ? " (Sicherungskopie)" : ""}`);
          main.append(el("span", "row-sub", `aus ${r.original} · ${date(r.deleted_at)} · ${size(r.size)} · noch ${r.days_left} Tage`));
          const btn = el("button", "btn btn--small", "Wiederherstellen");
          btn.type = "button";
          btn.addEventListener("click", async () => {
            const res = await invoke<ServerMessage>("trash_restore", { id: r.id });
            if (res.type === "reply" && !res.ok) showError(res.text);
            await loadTrash();
          });
          li.append(main, btn);
          return li;
        })
      : [el("li", "dim", "Der Papierkorb ist leer.")]),
  );
}
loaders.trash = () => loadTrash();
$("trash-open").addEventListener("click", () => invoke("open_place", { which: "trash" }).catch((e) => showError(String(e))));

// Zuletzt benutzten Reiter wieder öffnen
let start = "voice";
try {
  start = localStorage.getItem("panco-tab") ?? "voice";
} catch {}
selectTab(document.querySelector(`[data-tab="${start}"]`) ? start : "voice");
