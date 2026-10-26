// Vorschau- und Messseite für den geteilten Avatar.
// ?mode=none   – leere Seite (Basislinie)
// ?mode=static – Avatar ohne Animationen
// ?mode=anim   – Avatar wie in der App (Standard)
import "../../shared/tokens.css";
import "../../shared/avatar/panco.css";
import { PancoAvatar } from "../../shared/avatar/panco";
import svgMarkup from "../../shared/avatar/panco.svg?raw";

const mode = new URLSearchParams(location.search).get("mode") ?? "anim";
const stage = document.getElementById("stage")!;
const reduced = document.getElementById("reduced") as HTMLInputElement;

if (mode === "static") {
  // Nur das SVG, ohne Controller und ohne CSS-Animation.
  stage.innerHTML = `<div class="panco-wrap is-reduced">${svgMarkup}</div>`;
} else if (mode === "anim") {
  const avatar = new PancoAvatar(stage);
  reduced.addEventListener("change", () => avatar.setReduced(reduced.checked));
}

const style = document.createElement("style");
style.textContent = `
  body { margin: 0; min-height: 100vh; display: grid; place-items: center;
         background: radial-gradient(circle at 50% 35%, var(--pc-bg-1), var(--pc-bg-0) 70%);
         color: var(--pc-text-dim); font: var(--pc-font-size) var(--pc-font); }
  main { text-align: center; }
  #stage { width: 220px; margin: 0 auto 16px; }
`;
document.head.appendChild(style);
