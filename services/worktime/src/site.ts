// The public pages and the files they use, served from site/. Each file is named with a literal URL
// so that Vercel's file tracing ships it with the function.
export const ORIGIN = "https://worktime.thanaen.dev";

export interface Page {
  path: string;
  file: URL;
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
    title: "WorkTime · suivi du temps de travail pour Windows",
    description:
      "WorkTime compte votre temps de travail depuis la barre des tâches de Windows, sans rien saisir, et vous dit à quelle heure partir. Gratuit, licence MIT.",
    image: {
      path: "/og/home.png",
      alt: "WorkTime : « À quelle heure pourrez-vous partir aujourd’hui ? », à côté du panneau de la barre des tâches qui indique « Dans les temps », départ prévu à 16:54.",
    },
  },
];

// The statistics page is drawn per request from its template (stats-page.ts), with the same head.
export const STATISTICS: Page = {
  path: "/statistiques",
  file: new URL("../site/statistiques.html", import.meta.url),
  title: "Statistiques · WorkTime",
  description:
    "Combien d’installations de WorkTime sont actives chaque mois, par version et par mode d’installation : les chiffres du signal anonyme, publiés en données ouvertes.",
  image: PAGES[0]!.image,
};

const DAY = 86400;

// PostHog's managed proxy for the WorkTime project (README.md, "Pages").
export const ANALYTICS = "https://e.worktime.thanaen.dev";

const FILES: Record<string, { url: URL; type: string; maxAge: number }> = {
  "/fonts/IBMPlexSans-Regular.woff2": { url: new URL("../site/fonts/IBMPlexSans-Regular.woff2", import.meta.url), type: "font/woff2", maxAge: 7 * DAY },
  "/fonts/IBMPlexSans-Medium.woff2": { url: new URL("../site/fonts/IBMPlexSans-Medium.woff2", import.meta.url), type: "font/woff2", maxAge: 7 * DAY },
  "/fonts/IBMPlexSans-SemiBold.woff2": { url: new URL("../site/fonts/IBMPlexSans-SemiBold.woff2", import.meta.url), type: "font/woff2", maxAge: 7 * DAY },
  "/fonts/IBMPlexMono-Regular.woff2": { url: new URL("../site/fonts/IBMPlexMono-Regular.woff2", import.meta.url), type: "font/woff2", maxAge: 7 * DAY },
  "/fonts/IBMPlexMono-Medium.woff2": { url: new URL("../site/fonts/IBMPlexMono-Medium.woff2", import.meta.url), type: "font/woff2", maxAge: 7 * DAY },
  "/favicon.svg": { url: new URL("../site/favicon.svg", import.meta.url), type: "image/svg+xml", maxAge: DAY },
  "/favicon.ico": { url: new URL("../site/favicon.ico", import.meta.url), type: "image/x-icon", maxAge: DAY },
  "/apple-touch-icon.png": { url: new URL("../site/apple-touch-icon.png", import.meta.url), type: "image/png", maxAge: DAY },
  "/analytics.js": { url: new URL("../site/analytics.js", import.meta.url), type: "text/javascript", maxAge: 3600 },
  "/og/home.png": { url: new URL("../site/og/home.png", import.meta.url), type: "image/png", maxAge: DAY },
};

const escape = (text: string) =>
  text.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

// What search engines, link previews and browsers read about a page, and the audience measurement.
export function head(page: Page): string {
  const url = ORIGIN + page.path;
  const image = ORIGIN + page.image.path;
  return [
    `<title>${escape(page.title)}</title>`,
    `<meta name="description" content="${escape(page.description)}">`,
    `<link rel="canonical" href="${url}">`,
    `<meta property="og:type" content="website">`,
    `<meta property="og:site_name" content="WorkTime">`,
    `<meta property="og:locale" content="fr_FR">`,
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

const rendered = new Map<Page, Promise<string>>();

function render(page: Page): Promise<string> {
  let html = rendered.get(page);
  if (!html) {
    html = Bun.file(page.file)
      .text()
      .then((text) => text.replace("<!-- head -->", head(page)));
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

// One GET route per page and per file.
export function siteRoutes(): Record<string, { GET(): Response | Promise<Response> }> {
  const routes: Record<string, { GET(): Response | Promise<Response> }> = {};
  for (const page of PAGES) {
    routes[page.path] = {
      async GET() {
        return new Response(await render(page), {
          headers: { "content-type": "text/html; charset=utf-8", "cache-control": "public, max-age=300" },
        });
      },
    };
  }
  for (const [path, file] of Object.entries(FILES)) {
    routes[path] = {
      GET() {
        return new Response(Bun.file(file.url), {
          headers: { "content-type": file.type, "cache-control": `public, max-age=${file.maxAge}` },
        });
      },
    };
  }
  routes["/robots.txt"] = { GET: () => text(`User-agent: *\nDisallow: /v1/\nDisallow: /updates/\nDisallow: /download\nSitemap: ${ORIGIN}/sitemap.xml\n`, "text/plain") };
  routes["/sitemap.xml"] = { GET: () => text(sitemap(), "application/xml") };
  return routes;
}
