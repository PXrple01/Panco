// Rendert App- und Tray-Icons aus shared/avatar/panco.svg (Playwright/Chromium).
// Aufruf: node tools/icons/render-icons.cjs   → schreibt nach pc-app/src-tauri/icons/
const { execSync } = require("node:child_process");
const path = require("node:path");
const fs = require("node:fs");
const { chromium } = require(path.join(execSync("npm root -g").toString().trim(), "playwright"));

const root = path.join(__dirname, "../..");
const svg = fs.readFileSync(path.join(root, "shared/avatar/panco.svg"), "utf8");
const out = path.join(root, "pc-app/src-tauri/icons");
// Nur der Kopf (Ohren bis Mund) für kleine Größen.
const headSvg = svg.replace('viewBox="0 0 240 240"', 'viewBox="30 14 180 180"');

const pages = {
  "app-1024.png": { size: 1024, html: `<div style="width:1024px;height:1024px;border-radius:220px;background:radial-gradient(circle at 50% 38%,#2a1f45,#0a0a0f 72%);display:grid;place-items:center"><div style="width:860px">${svg}</div></div>` },
  "tray.png": { size: 32, html: `<div style="width:32px;height:32px">${headSvg}</div>` },
  "tray-off.png": { size: 32, html: `<div style="width:32px;height:32px;filter:grayscale(1) brightness(.75);opacity:.7">${headSvg}</div>` },
  "tray-listen.png": { size: 32, html: `<div style="width:32px;height:32px;border-radius:50%;background:radial-gradient(circle,#8b5cf6 0,#8b5cf600 70%)">${headSvg}</div>` },
  "tray-rec.png": { size: 32, html: `<div style="width:32px;height:32px;position:relative">${headSvg}<span style="position:absolute;right:0;bottom:0;width:11px;height:11px;border-radius:50%;background:#f43f5e;border:2px solid #fff"></span></div>` },
  "tray-muted.png": { size: 32, html: `<div style="width:32px;height:32px;position:relative;filter:grayscale(.6)">${headSvg}<span style="position:absolute;left:-2px;top:14px;width:36px;height:4px;background:#f87171;transform:rotate(-35deg);border-radius:2px"></span></div>` },
};

(async () => {
  const browser = await chromium.launch();
  for (const [file, { size, html }] of Object.entries(pages)) {
    const page = await browser.newPage({ viewport: { width: size, height: size }, deviceScaleFactor: 1 });
    await page.setContent(`<style>html,body{margin:0;background:transparent}svg{width:100%;height:auto;display:block}</style>${html}`);
    await page.screenshot({ path: path.join(out, file), omitBackground: true });
    await page.close();
  }
  await browser.close();
  console.log("Icons geschrieben nach", out);
})();
