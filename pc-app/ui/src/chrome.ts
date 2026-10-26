// Fenster-Chrom für Einstellungen und Ersteinrichtung: eigene Titelleiste, Licht nur bei sichtbarem
// und aktivem Fenster, gleitende Auswahl in der Seitenleiste, Abschnitte steigen beim Wechsel auf.
import { invoke } from "@tauri-apps/api/core";

const win = (action: string) => invoke("window_action", { action }).catch(() => {});

export function initChrome() {
  for (const el of document.querySelectorAll<HTMLElement>("[data-drag]")) {
    el.addEventListener("mousedown", (e) => {
      if (e.button === 0 && e.target === el) win("drag");
    });
    el.addEventListener("dblclick", (e) => {
      if (e.target === el) win("max");
    });
  }
  for (const b of document.querySelectorAll<HTMLButtonElement>("[data-win]")) {
    b.addEventListener("click", () => win(b.dataset.win!));
  }

  // Lichtkugeln und Puls nur, solange das Fenster sichtbar und aktiv ist (spart GPU/CPU).
  const sync = () =>
    document.body.classList.toggle("is-paused", document.visibilityState !== "visible" || !document.hasFocus());
  addEventListener("focus", sync);
  addEventListener("blur", sync);
  document.addEventListener("visibilitychange", sync);
  sync();

  // Lichtfleck folgt der Maus über den Karten (nur CSS-Variablen → transform).
  document.addEventListener(
    "pointermove",
    (e) => {
      const card = (e.target as Element | null)?.closest<HTMLElement>(".card");
      if (!card) return;
      const r = card.getBoundingClientRect();
      card.style.setProperty("--mx", `${Math.round(e.clientX - r.left)}px`);
      card.style.setProperty("--my", `${Math.round(e.clientY - r.top)}px`);
    },
    { passive: true },
  );
}

let firstTab = true;

/** Pille zum gewählten Reiter gleiten lassen und den Bereich einblenden. */
export function showTab(name: string) {
  const tab = document.querySelector<HTMLElement>(`.tab[data-tab="${name}"]`);
  const pill = document.querySelector<HTMLElement>(".nav__pill");
  if (tab && pill) {
    pill.classList.toggle("is-jump", firstTab);
    pill.style.height = `${tab.offsetHeight}px`;
    pill.style.setProperty("--pill-y", `${tab.offsetTop}px`);
    if (firstTab) requestAnimationFrame(() => requestAnimationFrame(() => pill.classList.remove("is-jump")));
  }
  firstTab = false;
  const panel = document.querySelector<HTMLElement>(`[data-panel="${name}"]`);
  if (panel) {
    [...panel.children].forEach((c, i) => (c as HTMLElement).style.setProperty("--i", String(i)));
    panel.classList.remove("is-entering");
    void panel.offsetWidth; // Animation neu starten
    panel.classList.add("is-entering");
  }
  document.getElementById("content")?.scrollTo({ top: 0 });
}
