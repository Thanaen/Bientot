// The /statistiques page: site/statistiques.html with its figures, drawn on the server as HTML and
// CSS bars, so that the page needs no script, like the homepage.
import type { Breakdown, PublicMonth, PublicStats } from "./stats";

const TEMPLATE = new URL("../site/statistiques.html", import.meta.url);

const CHANNELS: Record<string, { label: string; swatch: string }> = {
  direct: { label: "Programme d’installation du site", swatch: 'class="ch-direct"' },
  zip: { label: "Version ZIP", swatch: 'class="ch-zip"' },
  store: { label: "Microsoft&nbsp;Store", swatch: 'class="dotfill"' },
};

const escape = (text: string) =>
  text.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
const number = new Intl.NumberFormat("fr-FR");
const n = (value: number) => number.format(value);
const dateOf = (day: string) => new Date(`${day}T00:00:00Z`);
const format = (day: string, options: Intl.DateTimeFormatOptions) =>
  new Intl.DateTimeFormat("fr-FR", { timeZone: "UTC", ...options }).format(dateOf(day));
const monthName = (month: string) => format(`${month}-01`, { month: "long" });
const monthShort = (month: string) => format(`${month}-01`, { month: "short" });
const capitalize = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);
const isWeekend = (day: string) => [0, 6].includes(dateOf(day).getUTCDay());
const LESS = "moins de&nbsp;10";

// The top of the axis: 1, 2 or 5 times a power of ten, with at most four steps.
// When the tallest bar nearly reaches the last tick, the axis goes half a step higher, so that its
// value label stays off that gridline.
function axisTop(max: number): { top: number; step: number } {
  const target = Math.max(max, 10) / 4;
  const power = 10 ** Math.floor(Math.log10(target));
  const step = [1, 2, 5, 10].map((m) => m * power).find((s) => s >= target)!;
  const top = Math.ceil(Math.max(max, 1) / step) * step;
  return { top: max > top * 0.9 ? top + step / 2 : top, step };
}

interface Bar {
  value: number | null;
  x: string;
  label?: string;
  className?: string;
  colClass?: string;
  tick?: boolean;
  keepOnPhone?: boolean;
}

function bars(items: Bar[], aria: string, { height, gap }: { height: number; gap: number }): string {
  const { top, step } = axisTop(Math.max(0, ...items.map((b) => b.value ?? 0)));
  const ticks = Array.from({ length: Math.floor(top / step) + 1 }, (_, i) => i * step);
  const pc = (v: number) => `${((v / top) * 100).toFixed(2)}%`;
  const cols = items
    .map((b) => {
      const label = b.label === undefined ? "" : `<span class="val${b.value === null ? " low" : ""}">${b.label}</span>`;
      const classes = ["col", b.colClass, b.keepOnPhone ? "show-sm" : ""].filter(Boolean).join(" ");
      // A hidden value (under 10) gets a dashed stub, so that it does not read as zero.
      const bar = b.value === null ? "bar hidden" : `bar ${b.className ?? ""}`;
      return `<div class="${classes}"><div class="${bar}" style="height:${b.value === null ? "3px" : pc(b.value)}">${label}</div></div>`;
    })
    .join("");
  return `<div class="vbars${items.length > 12 ? " daily" : ""}" style="--h:${height}px;--gap:${gap}px">
        <div class="axis" aria-hidden="true">${ticks.map((t) => `<span style="bottom:${pc(t)}">${n(t)}</span>`).join("")}</div>
        <div class="plot" role="img" aria-label="${escape(aria)}">${ticks
          .slice(1)
          .map((t) => `<div class="grid" style="bottom:${pc(t)}"></div>`)
          .join("")}${cols}</div>
        <div class="xlab" aria-hidden="true">${items.map((b) => `<span${b.tick ? ' class="tick"' : ""}>${b.x}</span>`).join("")}</div>
      </div>`;
}

function kpis(stats: PublicStats, month: PublicMonth): string {
  const name = monthName(month.month);
  const previous = stats.months[stats.months.indexOf(month) - 1];
  let hint: string;
  if (month.partial) hint = `Mois en cours, jusqu’au ${format(stats.through, { day: "numeric", month: "long" })}`;
  else if (previous?.active && month.active) {
    const change = Math.round(((month.active - previous.active) / previous.active) * 100);
    const sign = change > 0 ? "+" : change < 0 ? "−" : "";
    hint = `<span${change > 0 ? ' class="up"' : ""}>${sign}${Math.abs(change)}&nbsp;%</span> par rapport à ${monthName(previous.month)}`;
  } else if (previous) hint = `${capitalize(monthName(previous.month))}&nbsp;: ${LESS}`;
  else hint = "Premier mois publié";

  const record = stats.days.filter((d) => d.day.startsWith(month.month)).reduce<{ day: string; installations: number } | null>(
    (best, d) => (d.installations !== null && (!best || d.installations > best.installations) ? { day: d.day, installations: d.installations } : best),
    null,
  );
  const value = (v: number | null) => (v === null ? `<span class="v small">${LESS}</span>` : `<span class="v">${n(v)}</span>`);
  return `<div class="kpis">
      <div class="kpi"><span class="l">Installations actives en ${name}</span>${value(month.active)}<span class="h">${hint}</span></div>
      <div class="kpi"><span class="l">Installations par jour en ${name}</span>${value(month.averageDaily)}<span class="h">En moyenne, week&#8209;ends compris</span></div>
      <div class="kpi"><span class="l">Record sur une journée en ${name}</span>${value(record?.installations ?? null)}<span class="h">${
        record ? capitalize(format(record.day, { weekday: "long", day: "numeric", month: "long" })) : "Aucun jour à 10 installations ou plus"
      }</span></div>
    </div>`;
}

function monthsChart(stats: PublicStats): string {
  const partial = stats.months.find((m) => m.partial);
  const days = partial ? Number(stats.through.slice(8)) : 0;
  const items = stats.months.map<Bar>((m, i) => ({
    value: m.active,
    x: monthShort(m.month),
    label: m.active === null ? LESS : m.partial ? `${n(m.active)}<span class="dd"> · ${days}&nbsp;j</span>` : n(m.active),
    className: m.partial ? "hatch" : "",
    keepOnPhone: i >= stats.months.length - 2,
  }));
  const aria = `Installations actives par mois : ${stats.months
    .map((m) => `${monthName(m.month)} ${m.month.slice(0, 4)}${m.partial ? " (en cours)" : ""} ${m.active ?? "moins de 10"}`)
    .join(", ")}.`;
  return `<section class="card" aria-labelledby="by-month">
      <div class="card-head"><h2 id="by-month">Installations actives par mois</h2><p>Au moins un signal dans le mois</p></div>
      ${bars(items, aria, { height: 200, gap: 16 })}
      <div class="legend"><span><i></i>Mois terminé</span>${
        partial ? `<span><i class="hatch"></i>${capitalize(monthName(partial.month))}, en cours (jusqu’au ${days})</span>` : ""
      }</div>
    </section>`;
}

function channelsCard(month: string, { shares, others }: Breakdown): string {
  const total = shares.reduce((sum, s) => sum + s.active, 0) + (others?.active ?? 0);
  const percent = (v: number) => `${Math.round((v / total) * 100)}&nbsp;%`;
  const rows = shares.map((s) => ({ ...(CHANNELS[s.key] ?? { label: escape(s.key), swatch: 'class="ch-zip"' }), active: s.active }));
  if (others?.active) rows.push({ label: "autres", swatch: 'class="ch-zip"', active: others.active });
  const noStore = !shares.some((s) => s.key === "store") && !others;
  return `<section class="card" aria-labelledby="by-channel">
        <div class="card-head"><h2 id="by-channel">Par mode d’installation</h2><p>${capitalize(monthName(month))} ${month.slice(0, 4)}</p></div>
        <div class="stack" role="img" aria-label="${escape(`Installations par mode d’installation en ${monthName(month)} ${month.slice(0, 4)} : ${rows.map((r) => `${r.label.replace("&nbsp;", " ")} ${r.active} (${Math.round((r.active / total) * 100)} %)`).join(", ")}.`)}">${rows
          .map((r) => `<i ${r.swatch} style="flex:${r.active}"></i>`)
          .join("")}</div>
        <div class="chlist">${rows
          .map((r) => `<div><i ${r.swatch}></i><span>${r.label}</span><span class="n">${n(r.active)}</span><span class="pc">${percent(r.active)}</span></div>`)
          .join("")}</div>${
          others && !others.active ? `\n        <p class="note">Les modes utilisés par moins de 10 installations ne sont pas montrés.</p>` : ""
        }${noStore ? `\n        <p class="note">Le Microsoft&nbsp;Store apparaîtra ici quand WorkTime y sera publié.</p>` : ""}
      </section>`;
}

function versionsCard(month: string, { shares, others }: Breakdown): string {
  const total = shares.reduce((sum, s) => sum + s.active, 0) + (others?.active ?? 0);
  const row = (label: string, active: number | null, other = false) =>
    `<div class="hrow${other ? " other" : ""}"><span class="lab">${label}</span><span class="track"><i style="width:${(((active ?? 0) / total) * 100).toFixed(1)}%"></i></span><span class="num">${
      active === null ? LESS : n(active)
    }</span></div>`;
  return `<section class="card" aria-labelledby="by-version">
        <div class="card-head"><h2 id="by-version">Par version</h2><p>${capitalize(monthName(month))} ${month.slice(0, 4)}</p></div>
        <div class="hbars">${shares.map((s) => row(escape(s.key), s.active)).join("")}${others ? row(`autres (${others.count})`, others.active, true) : ""}</div>
        <p class="note">Une version utilisée par moins de 10 installations rejoint «&nbsp;autres&nbsp;».</p>
      </section>`;
}

function daysChart(stats: PublicStats, month: string): string {
  const days = stats.days.filter((d) => d.day.startsWith(month));
  const peak = Math.max(0, ...days.map((d) => d.installations ?? 0));
  // Only the first day at the peak gets its value written above it.
  const peakDay = days.find((d) => d.installations !== null && d.installations === peak)?.day;
  const items = days.map<Bar>((d) => {
    const date = Number(d.day.slice(8));
    const isPeak = d.day === peakDay;
    return {
      value: d.installations,
      x: String(date),
      tick: date % 7 === 1,
      colClass: isWeekend(d.day) ? "we" : "",
      ...(isPeak ? { label: n(peak), keepOnPhone: true } : {}),
    };
  });
  const values = days.filter((d) => d.installations !== null);
  const aria = `Installations par jour en ${monthName(month)} ${month.slice(0, 4)}${
    values.length ? `, de ${Math.min(...values.map((d) => d.installations!))} à ${peak}` : ""
  }; les jours à moins de 10 installations ne sont pas montrés.`;
  return `<section class="card" aria-labelledby="by-day">
      <div class="card-head"><h2 id="by-day">Installations par jour en ${monthName(month)}</h2><p>En gris&nbsp;: samedis et dimanches${days.some((d) => d.installations === null) ? " · en pointillés&nbsp;: moins de&nbsp;10" : ""}</p></div>
      ${bars(items, aria, { height: 160, gap: 3 })}
    </section>`;
}

export function renderContent(stats: PublicStats): string {
  const reference = stats.reference;
  const stamp = `Données jusqu’au ${format(stats.through, { day: "numeric", month: "long", year: "numeric" })} <span class="nb">· mises à jour chaque nuit</span>`;
  const head = `<div class="wrap head">
    <p class="eyebrow">Statistiques publiques</p>
    <h1>WorkTime en chiffres</h1>
    <p class="lede">Chaque jour, WorkTime envoie un signal anonyme, que l’on peut couper dans Paramètres&nbsp;›&nbsp;Général. Voici ce que ces signaux permettent de compter, en accès libre et sans compte à créer.</p>
    <p class="stamp">${stamp}</p>
  </div>`;
  if (!reference || reference.month.active === null) {
    return `${head}
  <div class="wrap main">
    <section class="card empty"><h2>Moins de 10 installations pour l’instant</h2><p>Les chiffres apparaîtront ici dès que 10 installations auront envoyé leur signal dans le même mois. En dessous, ils pourraient désigner quelqu’un. Les fichiers de données, plus bas, sont déjà en ligne.</p></section>
  </div>`;
  }
  const month = reference.month.month;
  return `${head}
  <div class="wrap">${kpis(stats, reference.month)}</div>
  <div class="wrap main">
    ${monthsChart(stats)}
    <div class="two">
      ${channelsCard(month, reference.byChannel)}
      ${versionsCard(month, reference.byVersion)}
    </div>
    ${daysChart(stats, month)}
  </div>`;
}

export async function renderPage(stats: PublicStats): Promise<string> {
  return (await Bun.file(TEMPLATE).text()).replace("<!--content-->", renderContent(stats));
}
