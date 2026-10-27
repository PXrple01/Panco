// Overlay-Logik: Befehl tippen → Rust-Engine → Antwort als Sprechblase, ggf. Bestätigen.
import "../../../shared/tokens.css";
import "../../../shared/avatar/panco.css";
import "./overlay.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PancoAvatar } from "../../../shared/avatar/panco";
import type { AvatarState, Choice, ClientMessage, ConfirmRequest, ServerMessage, State } from "../../../shared/schema/messages";

interface UiSettings {
  reduce_motion: boolean;
  show_avatar: boolean;
  auto_hide_ms: number;
  enabled: boolean;
  state: State;
  /** Mini-Panco (per "Panco, …" geöffnet): kleines Icon + erkannter Text, keine Eingabezeile. */
  compact: boolean;
  /** Game Mode: Avatar schläft, keine Animationen. */
  game_mode: boolean;
  /** Antwort und erkannter Text, die eintrafen, während das Overlay noch lud. */
  pending: ServerMessage | null;
  heard: string | null;
}

const MIC_LABEL: Record<string, string> = {
  off: "Mikro aus",
  wake: "wartet auf „Panco“",
  recording: "hört zu …",
  muted: "Mikro stumm",
};

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const avatarHost = $<HTMLDivElement>("avatar");
const bubble = $<HTMLParagraphElement>("bubble");
const form = $<HTMLFormElement>("form");
const input = $<HTMLInputElement>("input");
const confirmRow = $<HTMLDivElement>("confirm");
const confirmHint = $<HTMLSpanElement>("confirm-hint");
const heard = $<HTMLParagraphElement>("heard");
const choicesBox = $<HTMLDivElement>("choices");
const activity = $<HTMLParagraphElement>("activity");
const detail = $<HTMLPreElement>("detail");
const mic = $<HTMLSpanElement>("mic");

const REST_AFTER_MS = 10_000;
let avatar: PancoAvatar | null = null;
let restTimer = 0;
let settings: Pick<UiSettings, "reduce_motion" | "show_avatar" | "auto_hide_ms" | "enabled"> = { reduce_motion: false, show_avatar: true, auto_hide_ms: 1600, enabled: true };
let lastAvatar: AvatarState = "idle";
let pendingConfirm: ConfirmRequest | null = null;
let hideTimer = 0;
let typeTimer = 0;
let counter = 0;
const history: string[] = [];
let historyPos = 0;

const card = $<HTMLDivElement>("card");
let mini = false;
let hasHeard = false;

/** Ausblenden, wenn alles erledigt ist. */
const hide = () => invoke("hide_overlay");
/** Wegklicken (✕/Esc): bricht auch Zuhören, Vorlesen und eine offene Rückfrage ab. */
const dismiss = () => invoke("overlay_dismiss");
const send = (msg: ClientMessage) => invoke<ServerMessage>("send", { msg }).then(handle).catch((e) => say(String(e), true));

/** Sprechblase mit Schreibmaschinen-Effekt (höchstens zwei Sätze, Text kommt kurz aus Rust). */
function say(text: string, isError = false) {
  window.clearInterval(typeTimer);
  bubble.hidden = false;
  bubble.classList.toggle("is-error", isError);
  // Animation neu starten
  bubble.style.animation = "none";
  void bubble.offsetWidth;
  bubble.style.animation = "";
  // Mini-Panco: ganzer Text sofort (die Fenstergröße folgt dem Text).
  if (settings.reduce_motion || mini) {
    bubble.textContent = text;
    fitSize();
    return;
  }
  let i = 0;
  bubble.textContent = "";
  typeTimer = window.setInterval(() => {
    bubble.textContent = text.slice(0, ++i);
    if (i >= text.length) {
      window.clearInterval(typeTimer);
      fitSize();
    }
  }, 18);
}

function react(mood: string) {
  avatarHost.className = "avatar";
  void avatarHost.offsetWidth;
  avatarHost.classList.add(`mood-${mood}`);
  if (!settings.show_avatar) avatarHost.classList.add("is-off");
}

/** Auswahlknöpfe (z. B. Suchtreffer). Ein Klick ruft die Aktion direkt auf. */
function showChoices(choices: Choice[] | undefined) {
  choicesBox.replaceChildren();
  choicesBox.hidden = !choices?.length;
  for (const c of choices ?? []) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "choice";
    const label = document.createElement("span");
    label.className = "choice__label";
    label.textContent = c.label;
    const detail = document.createElement("span");
    detail.className = "choice__detail";
    detail.textContent = c.detail ?? "";
    btn.append(label, detail);
    btn.addEventListener("click", () => {
      send({ type: "invoke", id: `o${++counter}`, action: c.action, params: c.params });
    });
    choicesBox.append(btn);
  }
  fitSize();
}

/** Fenster folgt der Karte (Rust rechnet die eingestellte Größe und Position ein). */
let lastSize = "";
new ResizeObserver(() => {
  const size = { width: card.offsetWidth, height: card.offsetHeight };
  const key = `${size.width}x${size.height}`;
  if (key === lastSize) return;
  lastSize = key;
  invoke("overlay_size", size).catch(() => {});
}).observe(card);

/** Mehrzeiliger Mini-Panco bekommt Ecken statt Pillenform. */
function fitSize() {
  requestAnimationFrame(() => card.classList.toggle("has-more", card.offsetHeight > 80));
}

/** Zeit zum Lesen: mindestens die eingestellte Anzeigedauer, bei langen Antworten länger. */
const readMs = (text: string) => Math.min(8000, Math.max(settings.auto_hide_ms, text.length * 55));

function handle(msg: ServerMessage) {
  // Zustandsmeldungen ("idle" nach der Antwort) dürfen das Ausblenden nicht abbrechen –
  // nur eine neue Interaktion (Wake-Word, Aufnahme, Antwort) tut das.
  if (msg.type !== "state" || msg.mic === "recording" || ["alert", "listening", "thinking"].includes(msg.avatar)) {
    window.clearTimeout(hideTimer);
  }
  wake();
  switch (msg.type) {
    case "stream":
      // Claude schreibt noch: Text wächst, "activity" zeigt sichtbar, was Panco gerade liest.
      window.clearInterval(typeTimer);
      bubble.hidden = !msg.text;
      bubble.classList.remove("is-error");
      bubble.textContent = msg.text;
      activity.hidden = !msg.activity;
      activity.textContent = msg.activity ? `Panco ${msg.activity} …` : "";
      if (msg.done) {
        activity.hidden = true;
        react("info");
        if (settings.auto_hide_ms > 0 && !pendingConfirm) {
          hideTimer = window.setTimeout(hide, Math.min(20_000, Math.max(settings.auto_hide_ms, msg.text.length * 60)));
        }
      }
      fitSize();
      break;
    case "reply":
      // Testlauf in der Sandbox: die offene Rückfrage zum Skript bleibt stehen.
      if (pendingConfirm && msg.action === "shell.test") {
        say(msg.text, !msg.ok);
        fitSize();
        break;
      }
      pendingConfirm = null;
      confirmRow.hidden = true;
      detail.hidden = true;
      activity.hidden = true;
      say(msg.text, !msg.ok && msg.mood === "error");
      react(msg.mood);
      showChoices(msg.choices);
      // Erfolg: lesen lassen, dann weg. Fehler: länger zeigen – aber ein per Sprache (ohne Fokus)
      // geöffnetes Overlay verschwindet immer von selbst, es soll kein Spiel verdecken.
      // Mit Auswahl bleibt das Overlay offen, bis man gewählt hat.
      if (settings.auto_hide_ms > 0 && !msg.choices?.length && (msg.ok || mini || !document.hasFocus())) {
        hideTimer = window.setTimeout(hide, msg.ok ? readMs(msg.text) : readMs(msg.text) * 2);
      }
      fitSize();
      break;
    case "confirm_request":
      pendingConfirm = msg;
      activity.hidden = true;
      say(msg.text);
      // Vorschau zum Prüfen: Diff einer Datei oder das Skript mit Panco-Einschätzung
      detail.hidden = !msg.detail;
      detail.replaceChildren(
        ...(msg.detail ?? "").split("\n").map((line) => {
          const span = document.createElement("span");
          span.textContent = line + "\n";
          if (line.startsWith("+ ")) span.className = "add";
          else if (line.startsWith("- ")) span.className = "del";
          else if (line.startsWith("— ") || line.startsWith("• ")) span.className = "note";
          return span;
        }),
      );
      confirmRow.hidden = false;
      confirmHint.textContent = msg.pc_only ? "nur per Klick" : "oder „ja“ tippen";
      showChoices(msg.choices);
      if (!mini) $<HTMLButtonElement>("yes").focus();
      break;
    case "state":
      showState(msg);
      break;
  }
}

/** Avatar-Zustand, Mikrofon-Anzeige und Pegel (Schallwellen) aktualisieren. */
function showState(msg: State) {
  settings.enabled = msg.enabled;
  if (!msg.enabled) say("Not-Aus aktiv.", true);
  avatar?.setState(msg.avatar);
  avatarHost.style.setProperty("--level", String(msg.level ?? 0));
  avatarHost.classList.toggle("is-hearing", msg.mic === "recording");
  mic.dataset.state = msg.mic;
  mic.title = MIC_LABEL[msg.mic] ?? "";
  // Mini-Panco: zeigen, dass er zuhört, bis der erkannte Text da ist.
  if (mini && !hasHeard && (msg.mic === "recording" || msg.avatar === "listening")) {
    heard.hidden = false;
    heard.classList.add("is-waiting");
    heard.textContent = "Ich höre zu …";
    fitSize();
  }
  // Per Sprache geöffnet, aber nichts gesagt → still wieder verschwinden.
  if (msg.avatar === "idle" && lastAvatar === "listening" && !document.hasFocus()) {
    hideTimer = window.setTimeout(hide, 1200);
  }
  lastAvatar = msg.avatar;
}

form.addEventListener("submit", (e) => {
  e.preventDefault();
  const text = input.value.trim();
  if (!text) return;
  history.push(text);
  historyPos = history.length;
  input.value = "";
  send({ type: "command", id: `o${++counter}`, text });
});

input.addEventListener("keydown", (e) => {
  // Pfeiltasten: letzte Befehle
  if (e.key === "ArrowUp" && historyPos > 0) input.value = history[--historyPos];
  else if (e.key === "ArrowDown" && historyPos < history.length) input.value = history[++historyPos] ?? "";
  else return;
  e.preventDefault();
});

for (const [id, accept] of [["yes", true], ["no", false]] as const) {
  $<HTMLButtonElement>(id).addEventListener("click", () => {
    if (pendingConfirm) send({ type: "confirm", confirm_id: pendingConfirm.confirm_id, accept });
    if (!mini) input.focus();
  });
}

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") dismiss();
});
$<HTMLButtonElement>("close").addEventListener("click", dismiss);
// Am Avatar ziehen verschiebt Panco; die Position wird gemerkt.
avatarHost.addEventListener("mousedown", (e) => {
  if (e.button === 0) invoke("overlay_drag").catch(() => {});
});

/** Von Mini-Panco zur vollen Karte (Strg+Leertaste, während er schon offen ist). */
function setMini(on: boolean) {
  mini = on;
  document.body.classList.toggle("is-mini", on);
  if (!on) input.focus();
  fitSize();
}

/** Avatar atmet nur, solange etwas passiert; nach 10 s Ruhe hält er still (CPU sparen). */
function wake() {
  avatar?.setResting(false);
  window.clearTimeout(restTimer);
  restTimer = window.setTimeout(() => avatar?.setResting(true), REST_AFTER_MS);
}
for (const ev of ["keydown", "pointermove", "pointerdown"]) window.addEventListener(ev, wake, { passive: true });

/** Was die Spracherkennung verstanden hat – zur Kontrolle klein über der Sprechblase. */
function showHeard(text: string) {
  hasHeard = true;
  heard.hidden = false;
  heard.classList.remove("is-waiting");
  heard.textContent = `„${text}“`;
  fitSize();
}

// Nachrichten, die nicht aus einer Eingabe hier stammen (Sprache, iPhone, Not-Aus).
const listening = Promise.all([
  listen<ServerMessage>("panco://message", (e) => handle(e.payload)),
  listen<string>("panco://heard", (e) => showHeard(e.payload)),
  listen<string>("panco://mode", (e) => setMini(e.payload === "mini")),
]);

(async () => {
  // Erst zuhören, dann Einstellungen holen: ab diesem Aufruf sendet Panco direkt (nichts geht verloren).
  await listening;
  const ui = await invoke<UiSettings>("ui_settings");
  settings = ui;
  // Im Spiel: keine Animationen, Panda schläft (spart GPU/CPU).
  const calm = settings.reduce_motion || ui.game_mode;
  document.documentElement.classList.toggle("reduce-motion", calm);
  if (settings.show_avatar) avatar = new PancoAvatar(avatarHost, { reduced: calm });
  else avatarHost.classList.add("is-off");
  if (ui.game_mode) avatar?.setResting(true);
  else wake();
  setMini(ui.compact);
  showState(ui.state);
  if (ui.heard) showHeard(ui.heard);
  if (ui.pending) handle(ui.pending);
  fitSize();
})();
