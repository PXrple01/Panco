// Reiter "Claude": Key (Windows-Tresor), Modus, Modell, Verlauf, gespeicherte Skripte.
import { invoke } from "@tauri-apps/api/core";

interface ClaudeState {
  enabled: boolean;
  has_key: boolean;
  model: string;
  effort: string;
  allow_screen_images: boolean;
  models: [string, string][];
  history: { role: string; text: string }[];
  scripts: { id: string; name: string; purpose: string; script: string }[];
  sandbox: boolean;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
let state: ClaudeState | null = null;

function status(text: string, ok: boolean) {
  const el = $("cl-key-status");
  el.textContent = text;
  el.className = `small ${ok ? "ok" : "error"}`;
}

async function save() {
  if (!state) return;
  try {
    await invoke("claude_settings", {
      enabled: $<HTMLInputElement>("cl-on").checked,
      model: $<HTMLSelectElement>("cl-model").value,
      effort: $<HTMLSelectElement>("cl-effort").value,
      allowScreenImages: $<HTMLInputElement>("cl-images").checked,
    });
  } catch (e) {
    status(String(e), false);
  }
  load();
}

async function load() {
  const s = (state = await invoke<ClaudeState>("claude_state"));
  const chip = $("cl-chip");
  chip.textContent = s.enabled ? "an" : s.has_key ? "Key da, Modus aus" : "kein Key";
  chip.className = s.enabled ? "chip chip--ok" : "chip";
  $("cl-key-del").hidden = !s.has_key;
  $<HTMLInputElement>("cl-key").placeholder = s.has_key ? "Key hinterlegt – zum Ändern neu eingeben" : "sk-ant-…";
  const on = $<HTMLInputElement>("cl-on");
  on.checked = s.enabled;
  on.disabled = !s.has_key;
  const model = $<HTMLSelectElement>("cl-model");
  model.replaceChildren(
    ...s.models.map(([id, label]) => {
      const o = document.createElement("option");
      o.value = id;
      o.textContent = label;
      o.selected = id === s.model;
      return o;
    }),
  );
  $<HTMLSelectElement>("cl-effort").value = s.effort;
  $<HTMLInputElement>("cl-images").checked = s.allow_screen_images;

  const chat = $("cl-history");
  chat.replaceChildren(
    ...s.history.map((h) => {
      const p = document.createElement("p");
      p.className = `msg msg--${h.role === "user" ? "me" : "panco"}`;
      p.textContent = h.text;
      return p;
    }),
  );
  if (!s.history.length) {
    const p = document.createElement("p");
    p.className = "dim small";
    p.textContent = "Noch nichts gefragt.";
    chat.append(p);
  }

  $("cl-sandbox").textContent = s.sandbox
    ? "Riskante Skripte kannst du vorher in der Windows-Sandbox testen."
    : "Tipp: Mit der Windows-Sandbox (Windows-Features) kann Panco riskante Skripte erst testen.";
  const list = $("cl-scripts");
  list.replaceChildren(
    ...s.scripts.map((sc) => {
      const card = document.createElement("div");
      card.className = "macro";
      const head = document.createElement("div");
      head.className = "macro__head";
      const name = document.createElement("strong");
      name.textContent = `„${sc.name}“ – ${sc.purpose}`;
      const del = document.createElement("button");
      del.type = "button";
      del.className = "btn btn--small";
      del.textContent = "Löschen";
      del.addEventListener("click", async () => {
        await invoke("script_remove", { id: sc.id }).catch(() => {});
        load();
      });
      head.append(name, del);
      const code = document.createElement("pre");
      code.className = "code";
      code.textContent = sc.script;
      card.append(head, code);
      return card;
    }),
  );
  if (!s.scripts.length) {
    const p = document.createElement("p");
    p.className = "dim small";
    p.textContent = "Noch keine gespeicherten Skripte.";
    list.append(p);
  }
}

$("cl-key-save").addEventListener("click", async () => {
  const input = $<HTMLInputElement>("cl-key");
  try {
    await invoke("claude_set_key", { key: input.value });
    input.value = "";
    status("Gespeichert (Windows-Tresor).", true);
  } catch (e) {
    status(String(e), false);
  }
  load();
});
$("cl-key-del").addEventListener("click", async () => {
  await invoke("claude_set_key", { key: "" });
  await invoke("claude_settings", {
    enabled: false,
    model: $<HTMLSelectElement>("cl-model").value,
    effort: $<HTMLSelectElement>("cl-effort").value,
    allowScreenImages: $<HTMLInputElement>("cl-images").checked,
  }).catch(() => {});
  status("Key entfernt.", true);
  load();
});
$("cl-key-test").addEventListener("click", async () => {
  status("Prüfe …", true);
  try {
    status(await invoke<string>("claude_test"), true);
  } catch (e) {
    status(String(e), false);
  }
});
for (const id of ["cl-on", "cl-model", "cl-effort", "cl-images"]) $(id).addEventListener("change", save);
$("cl-reset").addEventListener("click", async () => {
  await invoke("claude_reset");
  load();
});

document.addEventListener("panco-tab", (e) => {
  if ((e as CustomEvent<string>).detail === "claude") load().catch(() => {});
});
