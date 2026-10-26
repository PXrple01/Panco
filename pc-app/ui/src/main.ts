// Overlay-Logik: Befehl tippen → Rust-Engine → Antwort als Sprechblase, ggf. Bestätigen.
import "../../../shared/tokens.css";
import "../../../shared/avatar/panco.css";
import "./overlay.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PancoAvatar } from "../../../shared/avatar/panco";
import type { ClientMessage, ConfirmRequest, ServerMessage } from "../../../shared/schema/messages";

interface UiSettings {
  reduce_motion: boolean;
  show_avatar: boolean;
  auto_hide_ms: number;
  enabled: boolean;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const avatarHost = $<HTMLDivElement>("avatar");
const bubble = $<HTMLParagraphElement>("bubble");
const form = $<HTMLFormElement>("form");
const input = $<HTMLInputElement>("input");
const confirmRow = $<HTMLDivElement>("confirm");
const confirmHint = $<HTMLSpanElement>("confirm-hint");

const REST_AFTER_MS = 10_000;
let avatar: PancoAvatar | null = null;
let restTimer = 0;
let settings: UiSettings = { reduce_motion: false, show_avatar: true, auto_hide_ms: 1600, enabled: true };
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
      if (msg.ok && settings.auto_hide_ms > 0) hideTimer = window.setTimeout(hide, settings.auto_hide_ms);
      break;
    case "confirm_request":
      pendingConfirm = msg;
      say(msg.text);
      confirmRow.hidden = false;
      confirmHint.textContent = msg.pc_only ? "nur per Klick" : "oder „ja“ tippen";
      $<HTMLButtonElement>("yes").focus();
      break;
    case "state":
      settings.enabled = msg.enabled;
      if (!msg.enabled) say("Not-Aus aktiv.", true);
      break;
  }
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

(async () => {
  settings = await invoke<UiSettings>("ui_settings");
  document.documentElement.classList.toggle("reduce-motion", settings.reduce_motion);
  if (settings.show_avatar) avatar = new PancoAvatar(avatarHost, { reduced: settings.reduce_motion });
  else avatarHost.classList.add("is-off");
  wake();
  if (!settings.enabled) say("Not-Aus aktiv.", true);
  input.focus();
})();
