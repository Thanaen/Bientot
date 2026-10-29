// Every page the site can show, rendered to HTML with the service's own code: the homepage with and
// without a release version, the OG image's source, and the statistics page in each of its states,
// one per month of a made-up year so that every month and weekday name appears.
// Usage: bun scripts/fonts/pages.ts [service root] <output dir>
import { mkdir } from "node:fs/promises";
import { join, resolve } from "node:path";

const [root, out] = process.argv.length > 3 ? [resolve(process.argv[2]!), process.argv[3]!] : [resolve(import.meta.dir, "../.."), process.argv[2]!];
if (!out) throw new Error("usage: bun scripts/fonts/pages.ts [service root] <output dir>");

const { siteRoutes } = await import(join(root, "src/site.ts"));
const { renderPage } = await import(join(root, "src/stats-page.ts"));
const { publish } = await import(join(root, "src/stats.ts"));

type Day = { day: string; pings: number };
type Cell = { month: string; version: string; channel: string; active: number };

// A year that grows, with a quiet month, a month that drops, versions and channels under 10, and a
// channel the page has no label for. Each month's busiest day falls on a different weekday.
const year = 2027;
const days: Day[] = [];
const cells: Cell[] = [];
for (let m = 1; m <= 12; m++) {
  const month = `${year}-${String(m).padStart(2, "0")}`;
  const length = new Date(Date.UTC(year, m, 0)).getUTCDate();
  const scale = m === 7 ? 0.6 : m;
  for (let d = 1; d <= length; d++) {
    const date = new Date(Date.UTC(year, m - 1, d));
    const weekend = [0, 6].includes(date.getUTCDay());
    const pings = m === 2 ? (d % 3 ? 5 : 12) : 3500 * scale * (d === 8 + m ? 1.5 : weekend ? 0.3 : 1) + ((d * 37) % 90);
    days.push({ day: `${month}-${String(d).padStart(2, "0")}`, pings: Math.round(pings) });
  }
  if (m === 2) {
    cells.push({ month, version: "1.0.0", channel: "direct", active: 6 });
    continue;
  }
  cells.push(
    { month, version: `1.${m}.0`, channel: "direct", active: Math.round(1640 * scale) },
    { month, version: `1.${m}.0`, channel: "store", active: Math.round(5230 * scale) },
    { month, version: `1.${m - 1}.2`, channel: "zip", active: Math.round(120 * scale) },
    { month, version: `1.${m}.0-beta.3`, channel: "insider", active: 40 },
    { month, version: "0.9.12", channel: "direct", active: 4 },
    { month, version: "0.9.11", channel: "zip", active: 3 },
  );
}

const pages: Record<string, string> = {};
const routes = siteRoutes(async () => "1.12.0-beta.3");
pages["index.html"] = await (await routes["/"].GET(new Request("https://bientot.dev/"))).text();
const noVersion = siteRoutes(async () => null);
pages["index-sans-version.html"] = await (await noVersion["/"].GET(new Request("https://bientot.dev/"))).text();
pages["statistiques-vide.html"] = await renderPage(publish([], [], `${year}-01-05`));
pages["statistiques-un-mois-sous-10.html"] = await renderPage(publish(days.slice(31, 59), cells.filter((c) => c.month === `${year}-02`), `${year}-03-02`));
for (let m = 1; m <= 12; m++) {
  const through = m === 12 ? `${year + 1}-01-03` : `${year}-${String(m + 1).padStart(2, "0")}-03`;
  const until = (d: Day) => d.day <= through;
  pages[`statistiques-${String(m).padStart(2, "0")}.html`] = await renderPage(publish(days.filter(until), cells.filter((c) => c.month <= through.slice(0, 7)), through));
}
// Not a real page: the homepage's styles around text the site does not show today (italics, bold
// inside italics, letters outside French, CJK), to check how the fonts handle it.
const home = pages["index.html"]!;
pages["specimen.html"] = home.slice(0, home.indexOf("<body")) + `<body><main class="wrap" style="padding:32px 24px">
<h1>Specimen</h1>
<p>Texte courant : « Déjà l’été — où ça ? » … Œuvre, cœur, Ÿ, ½, ©, ¿ñ? 08:50–12:05, 7&nbsp;h&#8239;24.</p>
<p><em>Une phrase en italique, avec <b>du gras dans l’italique</b> et <strong>du gras</strong>.</em></p>
<p><b>Gras seul</b> · <span style="font-weight:500">Medium</span> · <span style="font-weight:600">SemiBold</span> · <span style="font-weight:300">Light</span> · <span style="font-weight:800">Extra bold</span></p>
<p>Latin étendu A : Āā Ăă Ąą Ćć Čč Ďď Ēē Ęę Ěě Ğğ Ħħ Īī İı Ĳĳ Ķķ Ĺĺ Łł Ńń Ňň Ŋŋ Ōō Őő Œœ Řř Śś Şş Šš Ţţ Ťť Ūū Ůů Űű Ŵŵ Ŷŷ Źź Żż Žž ſ</p>
<p>Latin-1 : ¡¢£¤¥¦§¨ª¬®¯°±²³´µ¶·¸¹º¼¾¿ÀÁÂÃÄÅÆÇÈËÌÍÎÏÐÑÒÓÕÖ×ØÙÚÛÜÝÞßãäåæðñòóõö÷øúüýþÿ</p>
<p>Hors Latin : € • ‹ › ‚ „ − ‐ ‒ … ‰ ™ → ≈ ∞</p>
<p class="mono">Code : const départ = "16:54"; // ça marche — été « ok » ½ Ąą Łł €</p>
<p class="mono" style="font-weight:500">Mono medium : <em>italique</em> Œœ ñ</p>
<p>CJK : 日本語のテキスト、中文文本、한국어 텍스트。</p>
<p style="hyphens:auto;width:9em" lang="fr">Anticonstitutionnellement, les développements internationalisation.</p>
</main></body></html>`;
// The OG image's source loads its fonts from ../fonts, so it sits one folder down.
pages["og/home.html"] = await Bun.file(join(root, "site/og/home.html")).text();

await mkdir(join(out, "og"), { recursive: true });
for (const [name, html] of Object.entries(pages)) await Bun.write(join(out, name), html);
console.log(Object.keys(pages).join("\n"));
