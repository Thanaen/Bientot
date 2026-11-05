// The public pages and the files they use, served from site/. Each file is named with a literal URL
// so that Vercel's file tracing ships it with the function.
import { FONTS } from "./fonts";

export const ORIGIN = "https://bientot.dev";

// The site's first address. Installed copies still read their feed and send their signal there,
// and those routes answer on it as they always did; only the pages move to ORIGIN for good.
export const OLD_HOST = "worktime.thanaen.dev";

// A page asked for on OLD_HOST: a 301 to the same address on ORIGIN, or null to serve it here.
// Vercel's CDN keys its cache by host, so a page cached for one host never answers the other.
export function moved(request: Request): Response | null {
  if (request.headers.get("host") !== OLD_HOST) return null;
  const { pathname, search } = new URL(request.url);
  return new Response(null, { status: 301, headers: { location: ORIGIN + pathname + search } });
}

export type Language = "fr" | "en";

export interface Page {
  path: string;
  file: URL;
  language: Language;
  title: string;
  description: string;
  // The image shown when the link is shared: a 1200 × 630 PNG listed in FILES.
  image: { path: string; alt: string };
}

// Every page carries `<!-- head -->` in its <head>; head() fills it, and the sitemap lists the page.
export const PAGES: Page[] = [
  {
    path: "/",
    file: new URL("../site/index.html", import.meta.url),
    language: "fr",
    title: "Bientôt · suivi gratuit des heures de travail pour Windows",
    description:
      "Bientôt compte vos heures de travail sous Windows 11, sans rien saisir, et vous dit à quelle heure partir. Gratuit, open source, sans compte et hors ligne.",
    image: {
      path: "/og/home.png",
      alt: "L’application Bientôt : « À quelle heure pourrez-vous partir aujourd’hui ? », à côté du panneau de la barre des tâches qui indique « Dans les temps », départ prévu à 16:54.",
    },
  },
  {
    path: "/en",
    file: new URL("../site/en.html", import.meta.url),
    language: "en",
    title: "Bientôt · free working hours tracker for Windows",
    description:
      "Bientôt counts your working hours on Windows 11 with nothing to type, and tells you what time you can leave. Free, open source, no account, works offline.",
    image: {
      path: "/og/home-en.png",
      alt: "“What time can you leave today?” next to the Bientôt taskbar panel showing “On track” and a projected departure of 4:54 PM.",
    },
  },
];

// The homepage in each language. Search engines show each reader the one in their language, and
// the English one to everyone else.
const HOMEPAGES: Record<Language | "x-default", string> = { fr: "/", en: "/en", "x-default": "/en" };

// The statistics page is drawn per request from its template (stats-page.ts), with the same head.
export const STATISTICS: Page = {
  path: "/statistiques",
  file: new URL("../site/statistiques.html", import.meta.url),
  language: "fr",
  title: "Statistiques · Bientôt",
  description:
    "Combien d’installations de Bientôt sont actives chaque mois, par version et par mode d’installation : les chiffres du signal anonyme, publiés en données ouvertes.",
  image: PAGES[0]!.image,
};

const DAY = 86400;

export const INDEXNOW_KEY = "fc1df1497d565dd2216861364de720c3";

// PostHog's managed proxy for the Bientôt project (README.md, "Pages").
export const ANALYTICS = "https://e.bientot.dev";

const FILES: Record<string, { url: URL; type: string; maxAge: number; immutable?: true }> = {
  // Named by their content (scripts/fonts/subset.py), so a new version is a new address.
  ...Object.fromEntries(Object.entries(FONTS).map(([path, url]) => [path, { url, type: "font/woff2", maxAge: 365 * DAY, immutable: true as const }])),
  "/favicon.svg": { url: new URL("../site/favicon.svg", import.meta.url), type: "image/svg+xml", maxAge: DAY },
  "/favicon.ico": { url: new URL("../site/favicon.ico", import.meta.url), type: "image/x-icon", maxAge: DAY },
  "/apple-touch-icon.png": { url: new URL("../site/apple-touch-icon.png", import.meta.url), type: "image/png", maxAge: DAY },
  "/analytics.js": { url: new URL("../site/analytics.js", import.meta.url), type: "text/javascript", maxAge: 3600 },
  "/og/home.png": { url: new URL("../site/og/home.png", import.meta.url), type: "image/png", maxAge: DAY },
  "/og/home-en.png": { url: new URL("../site/og/home-en.png", import.meta.url), type: "image/png", maxAge: DAY },
  // What the app is, in a few lines, for AI assistants and the search engines behind them (llmstxt.org).
  "/llms.txt": { url: new URL("../site/llms.txt", import.meta.url), type: "text/plain; charset=utf-8", maxAge: DAY },
  // The key that proves to IndexNow (Bing, Yandex…) that our submissions come from the site (scripts/indexnow.ts).
  [`/${INDEXNOW_KEY}.txt`]: { url: new URL("../site/indexnow-key.txt", import.meta.url), type: "text/plain; charset=utf-8", maxAge: DAY },
};

const escape = (text: string) =>
  text.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

// What search engines, link previews and browsers read about a page, and the audience measurement.
export function head(page: Page): string {
  const url = ORIGIN + page.path;
  const image = ORIGIN + page.image.path;
  const translated = Object.values(HOMEPAGES).includes(page.path);
  const locale = (language: Language) => (language === "fr" ? "fr_FR" : "en_US");
  return [
    `<title>${escape(page.title)}</title>`,
    `<meta name="description" content="${escape(page.description)}">`,
    `<link rel="canonical" href="${url}">`,
    ...(translated
      ? Object.entries(HOMEPAGES).map(([language, path]) => `<link rel="alternate" hreflang="${language}" href="${ORIGIN + path}">`)
      : []),
    `<meta property="og:type" content="website">`,
    `<meta property="og:site_name" content="Bientôt">`,
    `<meta property="og:locale" content="${locale(page.language)}">`,
    ...(translated ? [`<meta property="og:locale:alternate" content="${locale(page.language === "fr" ? "en" : "fr")}">`] : []),
    `<meta property="og:url" content="${url}">`,
    `<meta property="og:title" content="${escape(page.title)}">`,
    `<meta property="og:description" content="${escape(page.description)}">`,
    `<meta property="og:image" content="${image}">`,
    `<meta property="og:image:type" content="image/png">`,
    `<meta property="og:image:width" content="1200">`,
    `<meta property="og:image:height" content="630">`,
    `<meta property="og:image:alt" content="${escape(page.image.alt)}">`,
    `<meta name="twitter:card" content="summary_large_image">`,
    `<link rel="icon" href="/favicon.ico" sizes="48x48">`,
    `<link rel="icon" href="/favicon.svg" type="image/svg+xml">`,
    `<link rel="apple-touch-icon" href="/apple-touch-icon.png">`,
    `<meta name="theme-color" content="#fafafa" media="(prefers-color-scheme: light)">`,
    `<meta name="theme-color" content="#282c33" media="(prefers-color-scheme: dark)">`,
    // PostHog's library through its proxy on our domain, then its settings (site/analytics.js). Both
    // are deferred, so they run in this order once the page is parsed.
    `<script defer crossorigin="anonymous" src="${ANALYTICS}/static/array.js"></script>`,
    `<script defer src="/analytics.js"></script>`,
  ].join("\n");
}

// The note under the download buttons is about the installer, the second choice after the Microsoft
// Store: it names the version /download serves, filled in per request where the page says
// `<!-- version -->`.
export function versionNote(version: string | null, language: Language): string {
  const nb = (text: string) => `<span class="nb">${text}</span>`;
  if (language === "en") {
    const installer = version ? `The installer (version ${nb(version)})` : "The installer";
    return `${installer} keeps the app up to date. It isn’t signed yet, so Windows may ask you to confirm.`;
  }
  const installer = version ? `Le programme d’installation (version ${nb(version)})` : "Le programme d’installation";
  return `${installer} met l’application à jour automatiquement. Il n’est pas signé&nbsp;: Windows peut demander une confirmation.`;
}

// The HTML without its tags, or without all but its links; repeated until no tag is left to remove.
function withoutTags(html: string, keepLinks: boolean): string {
  const tag = keepLinks ? /<(?!\/?a[ >])[^>]*>/g : /<[^>]*>/g;
  for (let previous = ""; previous !== html; ) {
    previous = html;
    html = html.replace(tag, "");
  }
  return html;
}

// The text of a question or an answer: links kept in answers, with absolute addresses.
const plain = (html: string, keepLinks: boolean) =>
  withoutTags(
    html.replace(/<a [^>]*href="([^"]+)"[^>]*>/g, (_, href: string) => `<a href="${href.startsWith("/") ? ORIGIN + href : href}">`),
    keepLinks,
  )
    .replace(/&nbsp;/g, "\u00a0")
    .replace(/&amp;/g, "&");

// The questions of a page's `<section id="faq">`, as schema.org data, so that search engines and AI
// assistants read them as questions and answers. Drawn from the visible text so the two never differ.
export function faqData(html: string): string {
  const section = html.match(/<section id="faq"[^>]*>([\s\S]*?)<\/section>/)?.[1];
  if (!section) return "";
  const questions = [...section.matchAll(/<h3>([\s\S]*?)<\/h3><p>([\s\S]*?)<\/p>/g)].map(([, question, answer]) => ({
    "@type": "Question",
    name: plain(question!, false),
    acceptedAnswer: { "@type": "Answer", text: plain(answer!, true) },
  }));
  const data = { "@context": "https://schema.org", "@type": "FAQPage", mainEntity: questions };
  return `\n<script type="application/ld+json">${JSON.stringify(data).replace(/</g, "\\u003c")}</script>`;
}

const rendered = new Map<Page, Promise<string>>();

function render(page: Page): Promise<string> {
  let html = rendered.get(page);
  if (!html) {
    html = Bun.file(page.file)
      .text()
      .then((text) => text.replace("<!-- head -->", head(page) + faqData(text)));
    rendered.set(page, html);
  }
  return html;
}

const text = (body: string, type: string) =>
  new Response(body, { headers: { "content-type": `${type}; charset=utf-8`, "cache-control": `public, max-age=${DAY}` } });

export function sitemap(): string {
  const urls = [...PAGES, STATISTICS].map((page) => `  <url><loc>${ORIGIN}${page.path}</loc></url>`).join("\n");
  return `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls}\n</urlset>\n`;
}

// Browsers keep a page five minutes. Past that, Vercel's CDN still answers at once with the copy it
// has and asks the function for a fresh one behind it, so a visitor never waits for a cold function
// and GitHub: the site has too few visitors to keep either warm, and that wait was most of the time
// a first visit took to show anything. A deployment empties the CDN, so only the version note can
// be late, by one visit.
const PAGE_HEADERS = {
  "content-type": "text/html; charset=utf-8",
  "cache-control": "public, max-age=300",
  "vercel-cdn-cache-control": `max-age=300, stale-while-revalidate=${30 * DAY}`,
};

// One GET route per page and per file. `version` gives the latest release's version, or null.
export function siteRoutes(version: () => Promise<string | null>): Record<string, { GET(request: Request): Response | Promise<Response> }> {
  const routes: Record<string, { GET(request: Request): Response | Promise<Response> }> = {};
  for (const page of PAGES) {
    routes[page.path] = {
      async GET(request) {
        const redirect = moved(request);
        if (redirect) return redirect;
        const [html, latest] = await Promise.all([render(page), version()]);
        return new Response(html.replace("<!-- version -->", versionNote(latest, page.language)), { headers: PAGE_HEADERS });
      },
    };
  }
  for (const [path, file] of Object.entries(FILES)) {
    routes[path] = {
      GET() {
        return new Response(Bun.file(file.url), {
          headers: { "content-type": file.type, "cache-control": `public, max-age=${file.maxAge}${file.immutable ? ", immutable" : ""}` },
        });
      },
    };
  }
  routes["/robots.txt"] = { GET: () => text(`User-agent: *\nDisallow: /v1/\nDisallow: /updates/\nDisallow: /download\nSitemap: ${ORIGIN}/sitemap.xml\n`, "text/plain") };
  routes["/sitemap.xml"] = { GET: () => text(sitemap(), "application/xml") };
  return routes;
}
