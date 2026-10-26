/**
 * Panco-Avatar-Controller (geteilt von PC-Overlay und PWA).
 *
 * Hält sich bewusst klein: nur Timer für zufällige Momente (Blinzeln). Alle Bewegungen
 * selbst sind CSS. Läuft nichts, solange die Seite unsichtbar ist (Page Visibility API).
 */
import svgMarkup from "./panco.svg?raw";

export interface PancoOptions {
  /** Atmen/Schweben abschalten (Einstellung "Animationen reduzieren"). */
  reduced?: boolean;
}

export class PancoAvatar {
  readonly el: HTMLDivElement;
  private blinkTimer = 0;
  private running = false;
  private readonly onVisibility = () => this.syncVisibility();

  constructor(host: HTMLElement, opts: PancoOptions = {}) {
    this.el = document.createElement("div");
    this.el.className = "panco-wrap";
    this.el.innerHTML = svgMarkup;
    host.appendChild(this.el);
    this.setReduced(opts.reduced ?? false);
    document.addEventListener("visibilitychange", this.onVisibility);
    this.syncVisibility();
  }

  setReduced(reduced: boolean): void {
    this.el.classList.toggle("is-reduced", reduced);
  }

  /**
   * Ruhepause: Atmen anhalten, bis wieder etwas passiert (spart CPU, wenn das Fenster offen
   * herumsteht). Blinzeln läuft weiter – es kostet nur während der 140 ms.
   */
  setResting(resting: boolean): void {
    this.el.classList.toggle("is-resting", resting);
  }

  /** Entfernt den Avatar und alle Timer/Listener. */
  destroy(): void {
    this.stop();
    document.removeEventListener("visibilitychange", this.onVisibility);
    this.el.remove();
  }

  private syncVisibility(): void {
    if (document.hidden) this.stop();
    else this.start();
  }

  private start(): void {
    if (this.running) return;
    this.running = true;
    this.el.classList.remove("is-paused");
    this.scheduleBlink();
  }

  private stop(): void {
    this.running = false;
    this.el.classList.add("is-paused");
    window.clearTimeout(this.blinkTimer);
  }

  private scheduleBlink(): void {
    const delay = 3000 + Math.random() * 3000; // 3–6 s
    this.blinkTimer = window.setTimeout(() => {
      if (!this.running) return;
      this.blink();
      // Gelegentlich doppelt blinzeln wirkt lebendiger.
      if (Math.random() < 0.2) window.setTimeout(() => this.blink(), 260);
      this.scheduleBlink();
    }, delay);
  }

  private blink(): void {
    this.el.classList.add("is-blinking");
    window.setTimeout(() => this.el.classList.remove("is-blinking"), 140);
  }
}
