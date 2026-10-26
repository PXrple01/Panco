// Misst Größe und CPU-Last des Avatars im Chromium (gleiche Engine-Familie wie WebView2).
// Aufruf: npm run -w tools avatar:measure
//
// Methode: Für jeden Modus (leer / statisch / animiert) läuft die Seite N Sekunden.
// Gemessen wird die CPU-Zeit aller Browser-Prozesse (/proc, nur Linux) und die
// Main-Thread-Arbeit des Renderers (CDP Performance-Metriken).
// Hinweis: Headless-Chromium rendert ohne GPU – Werte sind daher eher zu hoch (worst case).
import { createServer } from "node:http";
import { readFile, readdir } from "node:fs/promises";
import { readFileSync, statSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { execSync } from "node:child_process";
import { createRequire } from "node:module";
import path from "node:path";

const require = createRequire(path.join(execSync("npm root -g").toString().trim(), "_"));
const { chromium } = require("playwright");

const here = path.dirname(new URL(import.meta.url).pathname);
const root = path.join(here, "out/avatar-preview");
const shared = path.join(here, "../shared");
const SECONDS = Number(process.env.SECONDS_PER_MODE ?? 20);

// --- Größen -------------------------------------------------------------------------
const kb = (n) => (n / 1024).toFixed(1) + " KB";
const sizeOf = (f) => {
  const buf = readFileSync(f);
  return { raw: buf.length, gz: gzipSync(buf).length };
};
const parts = ["avatar/panco.svg", "avatar/panco.css", "avatar/panco.ts"].map((p) => [p, sizeOf(path.join(shared, p))]);
const avatarRaw = parts.reduce((a, [, s]) => a + s.raw, 0);
const avatarGz = parts.reduce((a, [, s]) => a + s.gz, 0);
console.log("Größen (Ziel Avatar < 150 KB):");
for (const [p, s] of parts) console.log(`  ${p.padEnd(20)} ${kb(s.raw).padStart(9)}  (gzip ${kb(s.gz)})`);
console.log(`  ${"Avatar gesamt".padEnd(20)} ${kb(avatarRaw).padStart(9)}  (gzip ${kb(avatarGz)})`);
console.log(`  ${"Schrift Inter".padEnd(20)} ${kb(statSync(path.join(shared, "fonts/inter-latin-var.woff2")).size).padStart(9)}`);

// --- Mini-Webserver für den Build --------------------------------------------------
const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".woff2": "font/woff2" };
const server = createServer(async (req, res) => {
  const url = new URL(req.url, "http://x");
  const file = path.join(root, url.pathname === "/" ? "index.html" : url.pathname);
  try {
    const body = await readFile(file);
    res.writeHead(200, { "content-type": types[path.extname(file)] ?? "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/`;

// --- CPU-Zeit der Browser-Prozesse aus /proc ---------------------------------------
const CLK = Number(execSync("getconf CLK_TCK").toString());
async function treeCpuSeconds(rootPid) {
  const pids = (await readdir("/proc")).filter((d) => /^\d+$/.test(d));
  const info = new Map();
  for (const pid of pids) {
    try {
      const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
      const f = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
      const name = stat.slice(stat.indexOf("(") + 1, stat.lastIndexOf(")"));
      info.set(Number(pid), { name, ppid: Number(f[1]), cpu: (Number(f[11]) + Number(f[12])) / CLK });
    } catch {}
  }
  let total = 0;
  // Nur Chromium-Prozesse zählen (nicht den Playwright-Treiber).
  const walk = (pid) => {
    const i = info.get(pid);
    if (i && /chrom|headless/i.test(i.name)) total += i.cpu;
    for (const [p, c] of info) if (c.ppid === pid) walk(p);
  };
  walk(rootPid);
  return total;
}

const browser = await chromium.launch();
const browserPid = process.pid; // Wurzel des Prozessbaums: dieser Node-Prozess
const results = [];
for (const mode of ["none", "static", "anim"]) {
  const page = await browser.newPage({ viewport: { width: 480, height: 480 } });
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  await page.goto(`${base}?mode=${mode}`);
  await page.waitForTimeout(2000); // Einschwingen (Laden, Font)
  const metric = async () => Object.fromEntries((await cdp.send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]));
  const m0 = await metric();
  const c0 = browserPid ? await treeCpuSeconds(browserPid) : 0;
  const t0 = performance.now();
  await page.waitForTimeout(SECONDS * 1000);
  const wall = (performance.now() - t0) / 1000;
  const c1 = browserPid ? await treeCpuSeconds(browserPid) : 0;
  const m1 = await metric();
  results.push({
    mode,
    cpu: browserPid ? ((c1 - c0) / wall) * 100 : NaN,
    mainThread: ((m1.TaskDuration - m0.TaskDuration) / wall) * 100,
    heapMB: m1.JSHeapUsedSize / 1048576,
  });
  await page.close();
}
await browser.close();
server.close();

const label = { none: "leere Seite", static: "Avatar statisch", anim: "Avatar animiert" };
console.log(`\nCPU über ${SECONDS} s (Prozent eines Kerns, Headless-Chromium ohne GPU):`);
console.log("  Modus              Browser gesamt   Renderer-Main-Thread   JS-Heap");
for (const r of results) {
  console.log(`  ${label[r.mode].padEnd(18)} ${r.cpu.toFixed(2).padStart(8)} %       ${r.mainThread.toFixed(2).padStart(8)} %         ${r.heapMB.toFixed(1)} MB`);
}
const base0 = results[0].cpu;
console.log(`\nZusatzlast animiert vs. leer: ${(results[2].cpu - base0).toFixed(2)} % eines Kerns`);
