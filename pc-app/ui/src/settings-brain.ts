// Reiter "Befehle & Gedächtnis": eigene Befehle, Merksätze/Regeln, Rückfragen pro Aktion.
import { invoke } from "@tauri-apps/api/core";

interface MacroRow {
  id: string;
  name: string;
  phrases: string[];
  steps: string[];
  tier: number;
}
interface Item {
  id: string;
  text: string;
}
interface BrainState {
  macros: MacroRow[];
  facts: Item[];
  rules: Item[];
  confirmable: { id: string; title: string; skip: boolean }[];
  quiet_secs: number;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text;
  return e;
}

function button(text: string, cls: string, onClick: () => void) {
  const b = el("button", cls, text);
  b.type = "button";
  b.addEventListener("click", onClick);
  return b;
}

/** Bearbeitbare Karte für einen eigenen Befehl (neu: `m` ohne id). */
function macroCard(m: Partial<MacroRow>): HTMLElement {
  const card = el("div", "macro");
  const head = el("div", "macro__head");
  const name = el("input", "path-input macro__name");
  name.placeholder = "Name, z. B. Feierabend";
  name.maxLength = 40;
  name.value = m.name ?? "";
  const chip = el("span", m.tier && m.tier > 1 ? "chip chip--warn" : "chip", m.tier && m.tier > 1 ? "fragt einmal nach" : "sofort");
  head.append(name, chip);

  const phrases = el("input", "path-input");
  phrases.placeholder = "Weitere Formulierungen (optional, mit Komma): Schluss für heute, Gute Nacht";
  phrases.value = (m.phrases ?? []).join(", ");

  const steps = el("textarea");
  steps.spellcheck = false;
  steps.placeholder = "Ein Schritt pro Zeile, z. B.\nschließe Discord\nLautstärke 30\nsperr den PC";
  steps.value = (m.steps ?? []).join("\n");

  const error = el("p", "error");
  error.hidden = true;
  const actions = el("div", "actions");
  actions.append(
    button("Speichern", "btn btn--accent btn--small", async () => {
      try {
        await invoke("macro_save", {
          id: m.id ?? null,
          name: name.value,
          phrases: phrases.value.split(","),
          steps: steps.value,
        });
        load();
      } catch (e) {
        error.hidden = false;
        error.textContent = String(e);
      }
    }),
  );
  if (m.id) {
    actions.append(
      button("Ausprobieren", "btn btn--small", () => invoke("macro_try", { name: m.name })),
      button("Löschen", "btn btn--small", async () => {
        await invoke("macro_remove", { id: m.id }).catch((e) => ((error.hidden = false), (error.textContent = String(e))));
        load();
      }),
    );
  } else {
    actions.append(button("Verwerfen", "btn btn--small", () => card.remove()));
  }
  card.append(head, phrases, steps, error, actions);
  return card;
}

function itemRow(item: Item): HTMLLIElement {
  const li = el("li");
  li.append(el("span", "row-main", item.text));
  li.append(
    button("✕", "btn btn--small", async () => {
      await invoke("memory_remove", { id: item.id }).catch(() => {});
      load();
    }),
  );
  return li;
}

function quietLabel(secs: number) {
  return secs > 0 ? `Fragt gerade nicht nach – noch ${Math.ceil(secs / 60)} Min.` : "Fragt bei Stufe-2-Befehlen nach.";
}

async function load() {
  const s = await invoke<BrainState>("brain_state");
  const list = $("macro-list");
  list.replaceChildren(...s.macros.map(macroCard));
  if (!s.macros.length) list.append(el("p", "dim small", "Noch keine eigenen Befehle."));

  const facts = $("fact-list");
  facts.replaceChildren(...s.facts.map(itemRow));
  if (!s.facts.length) facts.append(el("li", "dim", "Noch nichts gemerkt."));
  const rules = $("rule-list");
  rules.replaceChildren(...s.rules.map(itemRow));
  if (!s.rules.length) rules.append(el("li", "dim", "Noch keine Regeln."));

  $("quiet-status").textContent = quietLabel(s.quiet_secs);
  $("quiet-on").hidden = s.quiet_secs > 0;
  $("quiet-off").hidden = s.quiet_secs === 0;

  const box = $("confirm-list");
  box.replaceChildren(
    ...s.confirmable.map((c) => {
      const label = el("label", "field row");
      label.append(el("span", "label", `${c.title} ohne Rückfrage`));
      const sw = el("input", "switch");
      sw.type = "checkbox";
      sw.checked = c.skip;
      sw.dataset.id = c.id;
      sw.addEventListener("change", () => {
        const ids = [...box.querySelectorAll<HTMLInputElement>("input.switch")].filter((x) => x.checked).map((x) => x.dataset.id);
        invoke("set_skip_confirm", { actions: ids });
      });
      label.append(sw);
      return label;
    }),
  );
}

$("macro-new").addEventListener("click", () => {
  const card = macroCard({});
  $("macro-list").prepend(card);
  card.querySelector("input")?.focus();
});
$("fact-add").addEventListener("click", async () => {
  const input = $<HTMLInputElement>("fact-new");
  if (!input.value.trim()) return;
  try {
    await invoke("fact_add", { text: input.value });
    input.value = "";
  } catch (e) {
    input.value = "";
    input.placeholder = String(e);
  }
  load();
});
$("quiet-on").addEventListener("click", async () => {
  await invoke("set_quiet", { minutes: 10 });
  load();
});
$("quiet-off").addEventListener("click", async () => {
  await invoke("set_quiet", { minutes: 0 });
  load();
});

document.addEventListener("panco-tab", (e) => {
  if ((e as CustomEvent<string>).detail === "brain") load().catch(() => {});
});

// --- Spotify (optional) ---
interface SpotifyStatus {
  client_id: string;
  connected: boolean;
}

async function loadSpotify() {
  const s = await invoke<SpotifyStatus>("spotify_status");
  const chip = $("sp-chip");
  chip.textContent = s.connected ? "verbunden" : "nicht verbunden";
  chip.className = s.connected ? "chip chip--ok" : "chip";
  const input = $<HTMLInputElement>("sp-client");
  if (!input.value) input.value = s.client_id;
  $("sp-connect").hidden = s.connected;
  $("sp-disconnect").hidden = !s.connected;
}

$("sp-connect").addEventListener("click", async () => {
  const btn = $<HTMLButtonElement>("sp-connect");
  const error = $("sp-error");
  error.hidden = true;
  btn.disabled = true;
  btn.textContent = "Warte auf den Browser …";
  try {
    await invoke("spotify_connect", { clientId: $<HTMLInputElement>("sp-client").value });
  } catch (e) {
    error.hidden = false;
    error.textContent = String(e);
  } finally {
    btn.disabled = false;
    btn.textContent = "Verbinden";
    loadSpotify().catch(() => {});
  }
});
$("sp-disconnect").addEventListener("click", async () => {
  await invoke("spotify_disconnect");
  loadSpotify().catch(() => {});
});

document.addEventListener("panco-tab", (e) => {
  if ((e as CustomEvent<string>).detail === "brain") loadSpotify().catch(() => {});
});
