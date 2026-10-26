// Overlay-Logik: Befehl tippen → Rust-Engine → Antwort als Sprechblase, ggf. Bestätigen.
import "../../../shared/tokens.css";
import "../../../shared/avatar/panco.css";
import "./overlay.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PancoAvatar } from "../../../shared/avatar/panco";
import type { AvatarState, ClientMessage, ConfirmRequest, ServerMessage, State } from "../../../shared/schema/messages";

interface UiSettings {
  reduce_motion: boolean;
  show_avatar: boolean;
  auto_hide_ms: number;
  enabled: boolean;
  state: State;
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
const mic = $<HTMLSpanElement>("mic");

const REST_AFTER_MS = 10_000;
let avatar: PancoAvatar | null = null;
let restTimer = 0;
let settings: Omit<UiSettings, "state"> = { reduce_motion: false, show_avatar: true, auto_hide_ms: 1600, enabled: true };
let lastAvatar: AvatarState = "idle";
let pendingConfirm: ConfirmRequest | null = null;
let hideTimer = 0;
let typeTimer = 0;
let counter = 0;
const history: string[] = [];
let historyPos = 0;

const hide = () => invoke("hide_overlay");
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
  if (settings.reduce_motion) {
    bubble.textContent = text;
    return;
  }
  let i = 0;
  bubble.textContent = "";
  typeTimer = window.setInterval(() => {
    bubble.textContent = text.slice(0, ++i);
    if (i >= text.length) window.clearInterval(typeTimer);
  }, 18);
}

function react(mood: string) {
  avatarHost.className = "avatar";
  void avatarHost.offsetWidth;
  avatarHost.classList.add(`mood-${mood}`);
  if (!settings.show_avatar) avatarHost.classList.add("is-off");
}

function handle(msg: ServerMessage) {
  window.clearTimeout(hideTimer);
  wake();
  switch (msg.type) {
    case "reply":
      pendingConfirm = null;
      confirmRow.hidden = true;
      say(msg.text, !msg.ok && msg.mood === "error");
      react(msg.mood);
      // Erfolg: kurz zeigen, dann weg. Fehler: länger zeigen – aber ein per Sprache (ohne Fokus)
      // geöffnetes Overlay verschwindet immer von selbst, es soll kein Spiel verdecken.
      if (settings.auto_hide_ms > 0 && (msg.ok || !document.hasFocus())) {
        hideTimer = window.setTimeout(hide, msg.ok ? settings.auto_hide_ms : settings.auto_hide_ms * 2.5);
      }
      break;
    case "confirm_request":
      pendingConfirm = msg;
      say(msg.text);
      confirmRow.hidden = false;
      confirmHint.textContent = msg.pc_only ? "nur per Klick" : "oder „ja“ tippen";
      $<HTMLButtonElement>("yes").focus();
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
    input.focus();
  });
}

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") hide();
});

/** Avatar atmet nur, solange etwas passiert; nach 10 s Ruhe hält er still (CPU sparen). */
function wake() {
  avatar?.setResting(false);
  window.clearTimeout(restTimer);
  restTimer = window.setTimeout(() => avatar?.setResting(true), REST_AFTER_MS);
}
for (const ev of ["keydown", "pointermove", "pointerdown"]) window.addEventListener(ev, wake, { passive: true });

// Nachrichten, die nicht aus einer Eingabe hier stammen (Sprache, iPhone, Not-Aus).
listen<ServerMessage>("panco://message", (e) => handle(e.payload));
// Was die Spracherkennung verstanden hat – zur Kontrolle klein über der Sprechblase.
listen<string>("panco://heard", (e) => {
  heard.hidden = false;
  heard.textContent = `„${e.payload}“`;
});

(async () => {
  const ui = await invoke<UiSettings>("ui_settings");
  settings = ui;
  document.documentElement.classList.toggle("reduce-motion", settings.reduce_motion);
  if (settings.show_avatar) avatar = new PancoAvatar(avatarHost, { reduced: settings.reduce_motion });
  else avatarHost.classList.add("is-off");
  wake();
  showState(ui.state);
  input.focus();
})();
