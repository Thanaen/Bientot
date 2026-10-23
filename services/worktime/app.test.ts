import { beforeEach, describe, expect, test } from "bun:test";
import { createApp } from "./src/app";
import { MemoryStore } from "./src/store";

let clock: Date;
let app: ReturnType<typeof createApp>;

beforeEach(() => {
  clock = new Date("2026-10-12T22:30:00Z");
  app = createApp({ store: new MemoryStore(), now: () => clock });
});

const ping = (body: unknown) =>
  app["/v1/ping"].POST(
    new Request("http://local/v1/ping", { method: "POST", body: typeof body === "string" ? body : JSON.stringify(body) }),
  );
describe("POST /v1/ping", () => {
  test.each(["store", "direct", "zip"])("accepts the signal of the %s channel", async (channel) => {
    const response = await ping({ version: "1.0.0", channel, first_this_month: true });
    expect(response.status).toBe(204);
  });

  test.each([
    ["an extra field", { version: "1.0.0", channel: "store", first_this_month: true, id: "abc" }],
    ["a missing field", { version: "1.0.0", channel: "store" }],
    ["free text as version", { version: "hello world", channel: "store", first_this_month: true }],
    ["an unknown channel", { version: "1.0.0", channel: "steam", first_this_month: true }],
    ["a string flag", { version: "1.0.0", channel: "zip", first_this_month: "yes" }],
    ["an array", [1, 2, 3]],
    ["invalid JSON", "{"],
  ])("rejects %s", async (_, body) => {
    expect((await ping(body)).status).toBe(400);
  });

  test("rejects an oversized body", async () => {
    expect((await ping("x".repeat(300))).status).toBe(413);
  });
});

describe("GET /updates/:file", () => {
  const get = (file: string) =>
    app["/updates/:file"].GET(Object.assign(new Request(`http://local/updates/${file}`), { params: { file } }));
  const releases = "https://github.com/Thanaen/WorkTime/releases";

  test("points at the latest release's feed, uncached", () => {
    const response = get("releases.win.json");
    expect(response.status).toBe(302);
    expect(response.headers.get("location")).toBe(`${releases}/latest/download/releases.win.json`);
    expect(response.headers.get("cache-control")).toBe("no-store");
  });

  test.each(["full", "delta"])("points at a %s package in the release of its version", (kind) => {
    const file = `Thanaen.WorkTime-0.2.0-alpha.13-${kind}.nupkg`;
    expect(get(file).headers.get("location")).toBe(`${releases}/download/v0.2.0-alpha.13/${file}`);
  });

  test.each(["..", ".hidden", "a/b", "a%2Fb", "", "RELEASES", "Thanaen.WorkTime-x-full.nupkg", "Other-1.0.0-full.nupkg"])(
    "finds no file named %p",
    (file) => {
      expect(get(file).status).toBe(404);
    },
  );

  test("/download points at the latest installer", () => {
    expect(app["/download"].GET().headers.get("location")).toBe(
      `${releases}/latest/download/Thanaen.WorkTime-win-Setup.exe`,
    );
  });
});

describe("the site", () => {
  // The site's routes are generated, so the app's type does not name them.
  const get = async (path: string) => {
    const routes: Record<string, unknown> = app;
    const route = routes[path] as { GET?: () => Response | Promise<Response> } | undefined;
    return route?.GET ? route.GET() : null;
  };
  const html = async () => (await get("/"))!.text();

  test("/ serves the page, which downloads through /download", async () => {
    const response = (await get("/"))!;
    expect(response.headers.get("content-type")).toBe("text/html; charset=utf-8");
    const page = await response.text();
    expect(page).toContain("<title>WorkTime");
    expect(page).toContain('href="/download"');
    expect(page).not.toContain("<!-- head -->");
  });

  test("the page describes itself for search engines and link previews", async () => {
    const page = await html();
    expect(page).toContain('<link rel="canonical" href="https://worktime.thanaen.dev/">');
    expect(page).toContain('<meta property="og:image" content="https://worktime.thanaen.dev/og/home.png">');
    expect(page).toContain('<meta name="twitter:card" content="summary_large_image">');
    expect(page).toContain(`<script defer crossorigin="anonymous" src="https://e.worktime.thanaen.dev/static/array.js"></script>`);
    expect(page).toContain(`<script defer src="/analytics.js"></script>`);
    expect(page).not.toContain("/_vercel/");
  });

  test("every local file the page links to is served", async () => {
    const paths = new Set(
      [...(await html()).matchAll(/(?:href|src|content)="(?:https:\/\/worktime\.thanaen\.dev)?(\/[\w./-]+\.\w+)"/g)].map(
        (match) => match[1] ?? "",
      ),
    );
    expect(paths).toContain("/og/home.png");
    expect(paths).toContain("/fonts/IBMPlexSans-Regular.woff2");
    expect(paths).toContain("/analytics.js");
    for (const path of paths) {
      const response = await get(path);
      expect(response?.status, path).toBe(200);
      expect((await response!.arrayBuffer()).byteLength, path).toBeGreaterThan(100);
    }
  });

  test("the share image is a 1200 × 630 PNG", async () => {
    const bytes = new DataView(await (await get("/og/home.png"))!.arrayBuffer());
    expect(bytes.getUint32(16)).toBe(1200);
    expect(bytes.getUint32(20)).toBe(630);
  });

  test("robots.txt points at the sitemap, which lists the page", async () => {
    expect(await (await get("/robots.txt"))!.text()).toContain("Sitemap: https://worktime.thanaen.dev/sitemap.xml");
    expect(await (await get("/sitemap.xml"))!.text()).toContain("<loc>https://worktime.thanaen.dev/</loc>");
  });

  test.each(["/fonts/OFL.txt", "/og/home.html", "/fonts/IBMPlexSans-Regular.ttf"])("serves no file at %p", async (path) => {
    expect(await get(path)).toBeNull();
  });
});

describe("the statistics", () => {
  const page = (headers: Record<string, string> = {}) =>
    app["/statistiques"].GET(new Request("http://local/statistiques", { headers }));
  const file = (name: string) =>
    app["/statistiques/:file"].GET(Object.assign(new Request(`http://local/statistiques/${name}`), { params: { file: name } }));
  const pings = async (count: number, channel = "direct", version = "0.2.0") => {
    for (let i = 0; i < count; i++) await ping({ version, channel, first_this_month: true });
  };

  test("shows nothing but a notice below 10 installations", async () => {
    await pings(9);
    clock = new Date("2026-10-13T08:00:00Z");
    const html = await (await page()).text();
    expect(html).toContain("Pas encore de chiffres");
    expect(html).not.toContain('class="kpis"');
  });

  test("leaves out the current day and caches until the next one", async () => {
    clock = new Date("2026-10-31T09:00:00Z");
    await pings(12);
    await pings(30, "zip", "0.1.0-alpha.250");
    clock = new Date("2026-11-01T09:00:00Z");
    await pings(15);
    const response = await page();
    expect(response.headers.get("cache-control")).toBe("public, max-age=600");
    // 09:00 to 00:05 the next day.
    expect(response.headers.get("vercel-cdn-cache-control")).toBe("max-age=54300, stale-while-revalidate=3600");
    const html = await response.text();
    expect(html).toContain("Installations actives en octobre");
    expect(html).toContain("Données jusqu’au 31 octobre 2026");
    expect(await (await file("quotidien.csv")).text()).toBe("jour,installations\n2026-10-31,42\n");
  });

  test("publishes a month once it is over", async () => {
    clock = new Date("2026-10-30T09:00:00Z");
    await pings(12);
    clock = new Date("2026-10-31T09:00:00Z");
    expect(await (await page()).text()).toContain("Pas encore de chiffres");
    expect(await (await file("mensuel.csv")).text()).toBe("mois,actives,moyenne_jour,pic_jour\n");
    expect(await (await file("quotidien.csv")).text()).toBe("jour,installations\n2026-10-30,12\n");
  });

  test("still shows the earlier months when the last one is under 10", async () => {
    clock = new Date("2026-09-15T09:00:00Z");
    await pings(100);
    clock = new Date("2026-10-15T09:00:00Z");
    await pings(5);
    clock = new Date("2026-11-01T09:00:00Z");
    const html = await (await page()).text();
    expect(html).toContain('class="kpis"');
    expect(html).toContain("Installations actives en octobre");
    expect(html).toContain("septembre 2026 100");
    expect(html).toContain("Septembre&nbsp;: 100");
    expect(html).not.toContain("Par mode d’installation");
  });

  test("serves the open data files, readable from other sites", async () => {
    clock = new Date("2026-10-31T09:00:00Z");
    await pings(12);
    clock = new Date("2026-11-01T09:00:00Z");
    const csv = await file("details.csv");
    expect(csv.headers.get("content-type")).toBe("text/csv; charset=utf-8");
    expect(csv.headers.get("access-control-allow-origin")).toBe("*");
    expect(await csv.text()).toBe("mois,par,valeur,actives\n2026-10,version,0.2.0,10\n2026-10,mode,direct,10\n");
    expect(await (await file("mensuel.json")).json()).toEqual([{ mois: "2026-10", actives: 12, moyenne_jour: null, pic_jour: 12 }]);
  });

  test("answers 304 to a browser that has the same day's figures", async () => {
    clock = new Date("2026-10-13T09:00:00Z");
    const etag = (await page()).headers.get("etag")!;
    expect((await page({ "if-none-match": etag })).status).toBe(304);
    expect((await page({ "if-none-match": `"other", W/${etag}` })).status).toBe(304);
    clock = new Date("2026-10-14T09:00:00Z");
    expect((await page({ "if-none-match": etag })).status).toBe(200);
  });

  test.each(["stats.csv", "mensuel.xml", "..", "mensuel.csv.bak"])("serves no file named %p", async (name) => {
    expect((await file(name)).status).toBe(404);
  });

  test("withholds a month without saying it is under 10", async () => {
    // A shown 10, a hidden 9 and 19 actives (the stats.test.ts case): March is withheld at 19.
    clock = new Date("2026-10-01T09:00:00Z");
    await pings(10);
    clock = new Date("2026-10-02T09:00:00Z");
    await pings(9);
    clock = new Date("2026-11-01T09:00:00Z");
    const alone = await (await page()).text();
    expect(alone).toContain("Pas encore de chiffres");
    expect(alone).not.toContain("Moins de 10");
  });

  test("labels a withheld month after a published one as not published", async () => {
    clock = new Date("2026-09-15T09:00:00Z");
    await pings(100);
    clock = new Date("2026-10-01T09:00:00Z");
    await pings(10);
    clock = new Date("2026-10-02T09:00:00Z");
    await pings(9);
    clock = new Date("2026-11-01T09:00:00Z");
    expect(await (await file("mensuel.csv")).text()).toBe("mois,actives,moyenne_jour,pic_jour\n2026-09,100,,100\n2026-10,,,10\n");
    const html = await (await page()).text();
    expect(html).toContain('Installations actives en octobre</span><span class="v small">non publié</span>');
    expect(html).toContain("octobre 2026 non publié");
  });

  test("the homepage links to the page, and the sitemap lists it", async () => {
    const routes: Record<string, { GET(): Response | Promise<Response> }> = app as never;
    expect(await (await routes["/"]!.GET()).text()).toContain('href="/statistiques"');
    expect(await (await routes["/sitemap.xml"]!.GET()).text()).toContain("<loc>https://worktime.thanaen.dev/statistiques</loc>");
  });

  test("the page carries the site's head", async () => {
    const html = await (await page()).text();
    expect(html).toContain("<title>Statistiques · WorkTime</title>");
    expect(html).toContain('<link rel="canonical" href="https://worktime.thanaen.dev/statistiques">');
    expect(html).toContain(`<script defer src="/analytics.js"></script>`);
    expect(html).not.toContain("<!-- head -->");
  });
});
