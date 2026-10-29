// Opens rendered pages (pages.ts) in Chromium, serving the site's files from a service root, and
// reports for each page the fonts it downloads and preloads and the characters each font face draws.
// With --shots <dir>, it also takes full-page screenshots for the pixel diffs (diff.py).
// Usage: bun scripts/fonts/browse.ts <pages dir> <service root> <report.json> [--shots <dir>] [--hinting full]
import { readdir } from "node:fs/promises";
import { join, resolve } from "node:path";
const { chromium } = require("playwright");

const [pagesDir, root, reportPath] = process.argv.slice(2).map((a) => resolve(a));
const shotsAt = process.argv.indexOf("--shots");
const shots = shotsAt > 0 ? resolve(process.argv[shotsAt + 1]!) : null;
if (!pagesDir || !root || !reportPath) throw new Error("usage: bun scripts/fonts/browse.ts <pages dir> <service root> <report.json> [--shots <dir>]");

const ORIGIN = "http://site.test";
const names = [...(await readdir(pagesDir)).filter((n) => n.endsWith(".html")), "og/home.html"];
const VIEWPORTS = [
  { name: "desktop", width: 1280, height: 800 },
  { name: "phone", width: 375, height: 740 },
];
const SCHEMES = ["light", "dark"] as const;

// Runs in the page: the text each text node draws, one line per node, grouped by its family, weight and style,
// with what CSS adds: pseudo-element text, the ellipsis of text-overflow, the hyphen of hyphens: auto.
function usage() {
  const out: Record<string, string> = {};
  const add = (style: CSSStyleDeclaration, text: string) => {
    if (!text) return;
    const t = style.textTransform;
    if (t === "uppercase") text = text.toUpperCase();
    else if (t === "lowercase") text = text.toLowerCase();
    else if (t === "capitalize") text = text.replace(/(^|\s)\S/g, (c) => c.toUpperCase());
    const key = `${style.fontFamily}|${style.fontWeight}|${style.fontStyle}`;
    out[key] = (out[key] ?? "") + text + "\n";
  };
  const visible = (el: Element) => (el as HTMLElement).getClientRects().length > 0 || el instanceof SVGElement;
  for (const el of document.querySelectorAll("body, body *")) {
    if (["SCRIPT", "STYLE", "TEMPLATE", "NOSCRIPT"].includes(el.tagName)) continue;
    const style = getComputedStyle(el);
    if (style.display === "none" || !visible(el)) continue;
    for (const node of el.childNodes) if (node.nodeType === Node.TEXT_NODE) add(style, (node.textContent ?? "").replace(/[\n\t\r]/g, " "));
    if (style.textOverflow === "ellipsis") add(style, "…");
    if (style.hyphens === "auto") add(style, "‐-");
    for (const pseudo of ["::before", "::after", "::marker"]) {
      const ps = getComputedStyle(el, pseudo);
      const m = /^"(.*)"$/.exec(ps.content);
      if (m && ps.display !== "none") add(ps, m[1]!);
    }
  }
  return out;
}

// --hinting full makes Chromium apply the fonts' TrueType hinting instructions, as Windows does;
// by default Chromium on Linux uses slight hinting, which ignores them.
const hintingAt = process.argv.indexOf("--hinting");
const browser = await chromium.launch(hintingAt > 0 ? { args: [`--font-render-hinting=${process.argv[hintingAt + 1]}`] } : {});
const report: Record<string, unknown> = {};
for (const name of names) {
  const entry = { fonts: {} as Record<string, number>, preloads: [] as string[], faces: {} as Record<string, string> };
  for (const viewport of VIEWPORTS) {
    for (const scheme of SCHEMES) {
      if (!shots && scheme === "dark") continue;
      const context = await browser.newContext({ viewport, colorScheme: scheme, deviceScaleFactor: 1 });
      const page = await context.newPage();
      await page.route("**/*", async (route: any) => {
        const url = new URL(route.request().url());
        if (url.origin !== ORIGIN) return route.abort();
        const file = url.pathname.startsWith("/fonts/") ? join(root, "site", url.pathname) : join(pagesDir, url.pathname === "/" ? "index.html" : url.pathname);
        const body = Bun.file(file);
        if (!(await body.exists())) return route.fulfill({ status: 404, body: "" });
        const bytes = Buffer.from(await body.arrayBuffer());
        if (url.pathname.startsWith("/fonts/")) entry.fonts[url.pathname] = bytes.length;
        return route.fulfill({ status: 200, body: bytes, contentType: url.pathname.endsWith(".woff2") ? "font/woff2" : "text/html; charset=utf-8" });
      });
      await page.goto(`${ORIGIN}/${name}`, { waitUntil: "networkidle" });
      await page.evaluate(() => document.fonts.ready);
      await page.waitForTimeout(150);
      entry.preloads = await page.$$eval('link[rel="preload"][as="font"]', (ls: HTMLLinkElement[]) => ls.map((l) => new URL(l.href).pathname));
      const faces: Record<string, string> = await page.evaluate(usage);
      // Each face's distinct lines of text, so that the subsetter can tell which ligatures are used.
      for (const [k, v] of Object.entries(faces)) entry.faces[k] = [...new Set([...(entry.faces[k] ?? "").split("\n"), ...v.split("\n")])].filter(Boolean).sort().join("\n");
      if (shots) await page.screenshot({ path: join(shots, `${name.replace("/", "_").replace(".html", "")}.${viewport.name}.${scheme}.png`), fullPage: true });
      await context.close();
    }
  }
  report[name] = entry;
}
await browser.close();
await Bun.write(reportPath, JSON.stringify(report, null, 1));
for (const [name, e] of Object.entries(report) as [string, any][]) {
  const total = Object.values(e.fonts as Record<string, number>).reduce((a, b) => a + b, 0);
  console.log(`${name}: ${total} B in ${Object.keys(e.fonts).length} fonts; preloads ${e.preloads.length}`);
}
