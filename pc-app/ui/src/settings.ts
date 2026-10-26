// Einstellungen & Diagnose. Fragt den Status nur ab, solange das Fenster sichtbar ist.
import "../../../shared/tokens.css";
import "../../../shared/avatar/panco.css";
import "./settings.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PancoAvatar } from "../../../shared/avatar/panco";

interface VoiceSettings {
  mode: "wake" | "push_to_talk" | "off";
  engine: "whisper" | "windows";
  model: string;
  keep_model_secs: number;
  wake_threshold: number;
  speak_replies: boolean;
  show_overlay: boolean;
  tts_voice: string;
  tts_pitch: number;
  tts_rate: number;
  barge_in: boolean;
}

type Anchor = "top_left" | "top_center" | "top_right" | "bottom_left" | "bottom_center" | "bottom_right" | "center" | "custom";
interface OverlayConfig {
  auto_hide_ms: number;
  reduce_motion: boolean;
  show_avatar: boolean;
  acrylic: boolean;
  compact: boolean;
  monitor: string;
  anchor: Anchor;
  custom: [number, number] | null;
  scale: number;
}
interface Appearance {
  overlay: OverlayConfig;
  monitors: { name: string; label: string }[];
  voices: { name: string; language: string; female: boolean }[];
}

interface Status {
  process: { own_mb: number; children_mb: number; cpu_seconds: number; children: string[] } | null;
  cpu_percent: number | null;
  enabled: boolean;
  muted: boolean;
  mic: "off" | "wake" | "recording" | "muted";
  wake_cpu_percent: number;
  whisper_loaded: boolean;
  wakeword_ready: boolean;
  last: { text: string; engine: string; audio_ms: number; load_ms: number; stt_ms: number } | null;
  models: { id: string; label: string; installed: boolean }[];
  voice: VoiceSettings;
  games: number;
  config_path: string;
  hotkeys: { overlay: string; kill_switch: string; push_to_talk: string; mic_mute: string };
}

/** "Ctrl+Alt+Space" → "Strg + Alt + Leertaste" */
const keyLabel = (k: string) =>
  k
    .split("+")
    .map((p) => ({ Ctrl: "Strg", Control: "Strg", Space: "Leertaste", End: "Ende", Shift: "Umschalt" })[p.trim()] ?? p.trim())
    .join(" + ");

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const fmt = (n: number, d = 1) => n.toLocaleString("de-DE", { minimumFractionDigits: d, maximumFractionDigits: d });
const MIC: Record<Status["mic"], string> = {
  off: "aus (geschlossen)",
  wake: "offen – nur das Wake-Word lauscht",
  recording: "nimmt einen Befehl auf",
  muted: "stumm",
};

let voice: VoiceSettings | null = null;
let downloading: string | null = null;
let modelsKey = "";
let pollTimer = 0;
let trainCount = 0;
let training = false;

function showError(msg: string | null) {
  const el = $("error");
  el.hidden = !msg;
  el.textContent = msg ?? "";
}

async function save(patch: Partial<VoiceSettings>) {
  if (!voice) return;
  voice = { ...voice, ...patch };
  try {
    await invoke("set_voice", { settings: voice });
    showError(null);
  } catch (e) {
    showError(String(e));
  }
}

function renderModels(s: Status) {
  const key = JSON.stringify(s.models) + s.voice.model + downloading;
  if (key === modelsKey) return; // nur neu zeichnen, wenn sich etwas geändert hat
  modelsKey = key;
  const list = $("models");
  list.replaceChildren();
  for (const m of s.models) {
    const li = document.createElement("li");
    const radio = document.createElement("input");
    radio.type = "radio";
    radio.name = "model";
    radio.checked = s.voice.model === m.id;
    radio.disabled = !m.installed;
    radio.addEventListener("change", () => save({ model: m.id }));
    const label = document.createElement("span");
    label.textContent = m.label;
    li.append(radio, label);
    if (m.installed) {
      const ok = document.createElement("span");
      ok.className = "ok small";
      ok.textContent = "installiert";
      li.append(ok);
    } else {
      const btn = document.createElement("button");
      btn.className = "btn";
      btn.type = "button";
      btn.textContent = downloading === m.id ? "lädt …" : "Herunterladen";
      btn.disabled = downloading !== null;
      btn.addEventListener("click", async () => {
        downloading = m.id;
        modelsKey = "";
        try {
          await invoke("download_model", { id: m.id });
        } catch (e) {
          downloading = null;
          showError(String(e));
        }
        refresh();
      });
      li.append(btn);
      if (downloading === m.id) {
        const bar = document.createElement("progress");
        bar.id = `progress-${m.id}`;
        bar.max = 1;
        bar.value = 0;
        li.append(bar);
      }
    }
    list.append(li);
  }
}

function render(s: Status) {
  const p = s.process;
  $("s-proc").textContent = p ? `${fmt(p.own_mb)} MB RAM (privat)` : "nicht messbar";
  $("s-web").textContent = p
    ? p.children.length
      ? `${fmt(p.children_mb)} MB in ${p.children.length} Prozess(en) – wird beim Schließen freigegeben`
      : "kein Fenster-Prozess"
    : "–";
  $("s-cpu").textContent = s.cpu_percent === null ? "misst …" : `${fmt(s.cpu_percent, 2)} % eines Kerns (inkl. dieses Fensters)`;
  $("s-mic").textContent = s.enabled ? MIC[s.mic] : "Not-Aus aktiv";
  $("s-wake").innerHTML = "";
  $("s-wake").append(
    s.wakeword_ready ? `eingesprochen · ${fmt(s.wake_cpu_percent, 2)} % eines Kerns beim Lauschen` : "noch nicht eingesprochen",
  );
  $("s-whisper").textContent = s.whisper_loaded
    ? `geladen (wird nach ${s.voice.keep_model_secs} s Ruhe freigegeben)`
    : s.models.some((m) => m.installed)
      ? "nicht geladen (0 MB)"
      : "kein Modell installiert – unten herunterladen";
  $("s-last").textContent = s.last
    ? `„${s.last.text}“ · ${s.last.engine} · Audio ${fmt(s.last.audio_ms / 1000)} s · Laden ${s.last.load_ms} ms · Erkennung ${s.last.stt_ms} ms`
    : "–";

  $("cfg-path").textContent = s.config_path;
  $("hk-overlay").textContent = keyLabel(s.hotkeys.overlay);
  $("hk-ptt").textContent = keyLabel(s.hotkeys.push_to_talk);
  $("hk-mute").textContent = keyLabel(s.hotkeys.mic_mute);
  $("hk-kill").textContent = keyLabel(s.hotkeys.kill_switch);
  $("headline").textContent = `${s.games} Steam-Spiele gefunden`;
  ($<HTMLInputElement>("muted")).checked = s.muted;

  // Formular nur beim ersten Mal füllen, danach gehört es dem Nutzer.
  if (!voice) {
    voice = s.voice;
    for (const el of document.querySelectorAll<HTMLInputElement>('input[name="mode"]')) el.checked = el.value === s.voice.mode;
    for (const el of document.querySelectorAll<HTMLInputElement>('input[name="engine"]')) el.checked = el.value === s.voice.engine;
    $<HTMLInputElement>("threshold").value = String(s.voice.wake_threshold);
    $("th-val").textContent = fmt(s.voice.wake_threshold, 2);
    $<HTMLInputElement>("speak").checked = s.voice.speak_replies;
    $<HTMLInputElement>("barge").checked = s.voice.barge_in;
    $<HTMLInputElement>("tts-pitch").value = String(s.voice.tts_pitch);
    $("tts-pitch-val").textContent = `${Math.round(s.voice.tts_pitch * 100)} %`;
    $<HTMLInputElement>("tts-rate").value = String(s.voice.tts_rate);
    $("tts-rate-val").textContent = `${Math.round(s.voice.tts_rate * 100)} %`;
    $<HTMLInputElement>("overlay").checked = s.voice.show_overlay;
    $<HTMLInputElement>("keep").value = String(s.voice.keep_model_secs);
  }
  renderModels(s);
  if (!training) $<HTMLButtonElement>("train-start").textContent = s.wakeword_ready ? "Neu einsprechen" : "Einsprechen starten";
}

async function refresh() {
  try {
    render(await invoke<Status>("status"));
  } catch (e) {
    showError(String(e));
  }
}

// Nur abfragen, solange das Fenster sichtbar ist.
function syncPolling() {
  window.clearInterval(pollTimer);
  const voiceTab = document.querySelector<HTMLElement>('[data-panel="voice"]');
  if (document.visibilityState === "visible" && !voiceTab?.hidden) {
    refresh();
    pollTimer = window.setInterval(refresh, 1000);
  }
}
document.addEventListener("visibilitychange", syncPolling);
// Nur abfragen, solange der Reiter "Sprache & Diagnose" offen ist.
document.addEventListener("panco-tab", syncPolling);

// ---- Formular ----
for (const el of document.querySelectorAll<HTMLInputElement>('input[name="mode"]'))
  el.addEventListener("change", () => save({ mode: el.value as VoiceSettings["mode"] }));
for (const el of document.querySelectorAll<HTMLInputElement>('input[name="engine"]'))
  el.addEventListener("change", () => save({ engine: el.value as VoiceSettings["engine"] }));
$<HTMLInputElement>("threshold").addEventListener("input", (e) => {
  $("th-val").textContent = fmt(Number((e.target as HTMLInputElement).value), 2);
});
$<HTMLInputElement>("threshold").addEventListener("change", (e) => save({ wake_threshold: Number((e.target as HTMLInputElement).value) }));
$<HTMLInputElement>("speak").addEventListener("change", (e) => save({ speak_replies: (e.target as HTMLInputElement).checked }));
$<HTMLInputElement>("barge").addEventListener("change", (e) => save({ barge_in: (e.target as HTMLInputElement).checked }));
for (const [id, key] of [["tts-pitch", "tts_pitch"], ["tts-rate", "tts_rate"]] as const) {
  const el = $<HTMLInputElement>(id);
  el.addEventListener("input", () => ($(`${id}-val`).textContent = `${Math.round(Number(el.value) * 100)} %`));
  el.addEventListener("change", () => save({ [key]: Number(el.value) }));
}
$<HTMLSelectElement>("tts-voice").addEventListener("change", (e) => save({ tts_voice: (e.target as HTMLSelectElement).value }));
$("tts-preview").addEventListener("click", async () => {
  await save({});
  invoke("tts_preview");
});

// ---- Aussehen: Position, Bildschirm, Größe ----
let look: OverlayConfig | null = null;

async function saveLook(patch: Partial<OverlayConfig>) {
  if (!look) return;
  look = { ...look, ...patch };
  try {
    await invoke("set_overlay", { overlay: look });
    showError(null);
  } catch (e) {
    showError(String(e));
  }
}

function option(value: string, label: string, selected: boolean) {
  const o = document.createElement("option");
  o.value = value;
  o.textContent = label;
  o.selected = selected;
  return o;
}

async function loadLook() {
  if (!voice) await refresh(); // füllt `voice` (Stimme, Tonhöhe …)
  const a = await invoke<Appearance>("appearance");
  look = a.overlay;
  $<HTMLInputElement>("ov-compact").checked = look.compact;
  $<HTMLSelectElement>("ov-monitor").replaceChildren(
    option("", "Hauptbildschirm", look.monitor === ""),
    ...a.monitors.map((m) => option(m.name, m.label, m.name === look!.monitor)),
  );
  $("ov-custom").hidden = look.anchor !== "custom";
  $<HTMLSelectElement>("ov-anchor").value = look.anchor;
  $<HTMLInputElement>("ov-scale").value = String(look.scale);
  $("ov-scale-val").textContent = `${Math.round(look.scale * 100)} %`;
  $<HTMLInputElement>("ov-hide").value = String(look.auto_hide_ms / 1000);
  $<HTMLInputElement>("ov-reduce").checked = look.reduce_motion;
  const current = voice?.tts_voice ?? "";
  $<HTMLSelectElement>("tts-voice").replaceChildren(
    option("", "Automatisch (deutsche Frauenstimme)", current === ""),
    ...a.voices.map((v) => option(v.name, `${v.name} · ${v.language}${v.female ? " · weiblich" : ""}`, v.name === current)),
  );
}
document.addEventListener("panco-tab", (e) => {
  if ((e as CustomEvent<string>).detail === "look") loadLook().catch((err) => showError(String(err)));
});
$<HTMLInputElement>("ov-compact").addEventListener("change", (e) => saveLook({ compact: (e.target as HTMLInputElement).checked }));
$<HTMLSelectElement>("ov-monitor").addEventListener("change", (e) => saveLook({ monitor: (e.target as HTMLSelectElement).value }));
$<HTMLSelectElement>("ov-anchor").addEventListener("change", (e) => saveLook({ anchor: (e.target as HTMLSelectElement).value as Anchor }));
$<HTMLInputElement>("ov-scale").addEventListener("input", (e) => {
  $("ov-scale-val").textContent = `${Math.round(Number((e.target as HTMLInputElement).value) * 100)} %`;
});
$<HTMLInputElement>("ov-scale").addEventListener("change", (e) => saveLook({ scale: Number((e.target as HTMLInputElement).value) }));
$<HTMLInputElement>("ov-hide").addEventListener("change", (e) =>
  saveLook({ auto_hide_ms: Math.round(Number((e.target as HTMLInputElement).value) * 1000) }),
);
$<HTMLInputElement>("ov-reduce").addEventListener("change", (e) => saveLook({ reduce_motion: (e.target as HTMLInputElement).checked }));
$("ov-preview").addEventListener("click", () => invoke("overlay_preview"));
$<HTMLInputElement>("overlay").addEventListener("change", (e) => save({ show_overlay: (e.target as HTMLInputElement).checked }));
$<HTMLInputElement>("keep").addEventListener("change", (e) => save({ keep_model_secs: Number((e.target as HTMLInputElement).value) }));
$<HTMLInputElement>("muted").addEventListener("change", (e) => invoke("set_muted", { muted: (e.target as HTMLInputElement).checked }));

// ---- Modell-Download ----
listen<{ id: string; done: number; total: number; finished: boolean; error: string | null }>("panco://download", (e) => {
  const d = e.payload;
  const bar = document.getElementById(`progress-${d.id}`) as HTMLProgressElement | null;
  if (bar && d.total) bar.value = d.done / d.total;
  if (d.finished) {
    downloading = null;
    modelsKey = "";
    showError(d.error);
    refresh();
  }
});

// ---- Wake-Word-Training ----
const NEEDED = 6;
function drawDots() {
  const dots = $("train-dots");
  dots.replaceChildren(...Array.from({ length: NEEDED }, (_, i) => {
    const s = document.createElement("span");
    if (i < trainCount) s.className = "done";
    return s;
  }));
}
function setTraining(on: boolean) {
  training = on;
  $<HTMLButtonElement>("train-start").hidden = on;
  $<HTMLButtonElement>("train-cancel").hidden = !on;
}
function askSample() {
  $("train-status").textContent = `Sag jetzt deutlich „Panco“ (${trainCount + 1}/${NEEDED}) …`;
  invoke("train_start");
}
$("train-start").addEventListener("click", () => {
  trainCount = 0;
  drawDots();
  setTraining(true);
  askSample();
});
$("train-cancel").addEventListener("click", () => {
  invoke("train_cancel");
  setTraining(false);
  trainCount = 0;
  drawDots();
  $("train-status").textContent = "Abgebrochen.";
});
listen<{ count: number; needed: number; done: boolean; error: string | null }>("panco://train", (e) => {
  const t = e.payload;
  if (t.done) {
    setTraining(false);
    $("train-status").textContent = t.error ? `Fehler: ${t.error}` : "Fertig! Sag jetzt „Panco, …“ – Panco hört dich.";
    refresh();
    return;
  }
  trainCount = t.count;
  drawDots();
  if (t.count < t.needed) window.setTimeout(askSample, 500);
  else $("train-status").textContent = "Berechne …";
});
listen<string>("panco://voice-error", (e) => {
  if (training) {
    $("train-status").textContent = e.payload;
    window.setTimeout(askSample, 1200);
  } else showError(e.payload);
});

new PancoAvatar($("avatar"), { reduced: true });
drawDots();
syncPolling();
