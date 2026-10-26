// Ordner-Formular (Papierkorb, Backup, Notizen, freigegebene Ordner) – genutzt von
// Ersteinrichtung und Einstellungen.
import { open } from "@tauri-apps/plugin-dialog";

export interface FileSettings {
  allowed_folders: string[];
  notes_dir: string;
  trash_dir: string;
  trash_days: number;
  backup_enabled: boolean;
  backup_dir: string;
  backup_keep: number;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text;
  return e;
}

async function pick(current: string): Promise<string | null> {
  const dir = await open({ directory: true, multiple: false, defaultPath: current || undefined });
  return typeof dir === "string" ? dir : null;
}

/** Zeile: Pfad + "Wählen …" */
function pathRow(value: string, onChange: (v: string) => void): HTMLDivElement {
  const row = el("div", "path-row");
  const input = el("input", "path-input");
  input.value = value;
  input.spellcheck = false;
  input.addEventListener("change", () => onChange(input.value.trim()));
  const btn = el("button", "btn", "Wählen …");
  btn.type = "button";
  btn.addEventListener("click", async () => {
    const dir = await pick(input.value);
    if (dir) {
      input.value = dir;
      onChange(dir);
    }
  });
  row.append(input, btn);
  return row;
}

function numberField(label: string, value: number, min: number, max: number, onChange: (v: number) => void) {
  const wrap = el("label", "field row");
  const span = el("span", "label", label);
  const input = el("input", "num");
  input.type = "number";
  input.min = String(min);
  input.max = String(max);
  input.value = String(value);
  input.addEventListener("change", () => onChange(Number(input.value)));
  wrap.append(span, input);
  return wrap;
}

function section(title: string, hint: string, ...children: HTMLElement[]) {
  const s = el("section", "card");
  s.append(el("h2", "", title));
  if (hint) s.append(el("p", "dim small", hint));
  s.append(...children);
  return s;
}

/** Baut das Formular in `host` und gibt eine Funktion zurück, die den aktuellen Stand liefert. */
export function folderForm(host: HTMLElement, initial: FileSettings): () => FileSettings {
  const f: FileSettings = structuredClone(initial);
  host.replaceChildren();

  host.append(
    section(
      "Papierkorb",
      "Panco löscht nie endgültig. „Gelöschtes“ landet hier und wird erst nach der Frist entfernt.",
      pathRow(f.trash_dir, (v) => (f.trash_dir = v)),
      numberField("Aufbewahren (Tage)", f.trash_days, 1, 365, (v) => (f.trash_days = v)),
    ),
  );

  const backupToggle = el("label", "field row");
  const cb = el("input", "switch");
  cb.type = "checkbox";
  cb.checked = f.backup_enabled;
  cb.addEventListener("change", () => (f.backup_enabled = cb.checked));
  backupToggle.append(el("span", "label", "Tägliches Backup"), cb);
  host.append(
    section(
      "Backup",
      "Einstellungen, Notizen, Wake-Word und Protokoll werden täglich gesichert. Tipp: ein anderes Laufwerk oder ein Cloud-Ordner (z. B. OneDrive).",
      backupToggle,
      pathRow(f.backup_dir, (v) => (f.backup_dir = v)),
      numberField("Backups behalten", f.backup_keep, 1, 60, (v) => (f.backup_keep = v)),
    ),
  );

  host.append(section("Notizen", "Hier legt Panco deine Notizen als Markdown-Dateien ab.", pathRow(f.notes_dir, (v) => (f.notes_dir = v))));

  const list = el("ul", "folder-list");
  const draw = () => {
    list.replaceChildren();
    f.allowed_folders.forEach((dir, i) => {
      const li = el("li");
      li.append(el("span", "path", dir));
      const rm = el("button", "btn btn--small", "Entfernen");
      rm.type = "button";
      rm.addEventListener("click", () => {
        f.allowed_folders.splice(i, 1);
        draw();
      });
      li.append(rm);
      list.append(li);
    });
  };
  draw();
  const add = el("button", "btn", "Ordner hinzufügen …");
  add.type = "button";
  add.addEventListener("click", async () => {
    const dir = await pick("");
    if (dir && !f.allowed_folders.includes(dir)) {
      f.allowed_folders.push(dir);
      draw();
    }
  });
  host.append(
    section(
      "Freigegebene Ordner",
      "Hier darf Panco ohne Nachfrage suchen, öffnen und anlegen. Löschen, Verschieben und Umbenennen fragt immer nach; außerhalb dieser Ordner auch das Öffnen.",
      list,
      add,
    ),
  );
  return () => structuredClone(f);
}
