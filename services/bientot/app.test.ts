import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { createApp } from "./src/app";
import { FONTS } from "./src/fonts";
import { MemoryStore } from "./src/store";

let clock: Date;
let app: ReturnType<typeof createApp>;

beforeEach(() => {
  clock = new Date("2026-10-12T22:30:00Z");
  app = createApp({ store: new MemoryStore(), now: () => clock, latest: async () => "0.2.0-alpha.53" });
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
  const releases = "https://github.com/Thanaen/Bientot/releases";

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

  test.each(["Thanaen.WorkTime-win-Setup.exe", "Thanaen.WorkTime-win-Portable.zip", "Bientot-Setup.exe", "Bientot-Portable.zip"])(
    "points at the latest release's %s",
    (file) => {
      expect(get(file).headers.get("location")).toBe(`${releases}/latest/download/${file}`);
    },
  );

  test("/download points at the latest installer under the app's name", () => {
    expect(app["/download"].GET().headers.get("location")).toBe(
      `${releases}/latest/download/Bientot-Setup.exe`,
    );
  });
});

describe("the site", () => {
  // The site's routes are generated, so the app's type does not name them.
  const get = async (path: string) => {
    const routes: Record<string, unknown> = app;
    const route = routes[path] as { GET?: (request: Request) => Response | Promise<Response> } | undefined;
    return route?.GET ? route.GET(new Request(`http://local${path}`)) : null;
  };
  const html = async () => (await get("/"))!.text();

  test("/ serves the page, which offers the Microsoft Store first and the installer through /download", async () => {
    const response = (await get("/"))!;
    expect(response.headers.get("content-type")).toBe("text/html; charset=utf-8");
    const page = await response.text();
    expect(page).toContain("<title>Bientôt");
    expect(page.indexOf('href="https://apps.microsoft.com/detail/9N4FL1HRF4KL"')).toBeGreaterThan(-1);
    expect(page.indexOf('href="https://apps.microsoft.com/detail/9N4FL1HRF4KL"')).toBeLessThan(page.indexOf('href="/download"'));
    expect(page).not.toContain("<!-- head -->");
  });

  test("the page names the latest release's version", async () => {
    const page = await html();
    expect(page).toContain('<p class="btn-note">Le programme d’installation (version <span class="nb">0.2.0-alpha.53</span>) met l’application à jour automatiquement.');
    expect(page).not.toContain("<!-- version -->");
  });

  test("the page leaves the version out while GitHub does not tell it, and asks again a minute later", async () => {
    const answers = [null, "0.2.0-alpha.54"];
    let asked = 0;
    app = createApp({ store: new MemoryStore(), now: () => clock, latest: async () => answers[asked++] ?? null });
    expect(await html()).toContain('<p class="btn-note">Le programme d’installation met l’application à jour automatiquement.');
    await html();
    expect(asked).toBe(1);
    clock = new Date(clock.getTime() + 60_000);
    expect(await html()).toContain(`(version <span class="nb">0.2.0-alpha.54</span>)`);
    clock = new Date(clock.getTime() + 300_000);
    await html();
    expect(asked).toBe(2);
  });

  test("the page keeps the version it knows while GitHub is slow or does not answer", async () => {
    let answer: (version: string | null) => void = () => {};
    let asked = 0;
    app = createApp({
      store: new MemoryStore(),
      now: () => clock,
      latest: () => (asked++ === 0 ? Promise.resolve("0.2.0-alpha.53") : new Promise((resolve) => (answer = resolve))),
    });
    expect(await html()).toContain(`(version <span class="nb">0.2.0-alpha.53</span>)`);
    clock = new Date(clock.getTime() + 600_000);
    expect(await html()).toContain(`(version <span class="nb">0.2.0-alpha.53</span>)`);
    expect(asked).toBe(2);
    answer(null);
    await Bun.sleep(0);
    expect(await html()).toContain(`(version <span class="nb">0.2.0-alpha.53</span>)`);
  });

  test("the page describes itself for search engines and link previews", async () => {
    const page = await html();
    expect(page).toContain('<link rel="canonical" href="https://bientot.dev/">');
    expect(page).toContain('<meta property="og:image" content="https://bientot.dev/og/home.png">');
    expect(page).toContain('<meta name="twitter:card" content="summary_large_image">');
    expect(page).toContain(`<script defer crossorigin="anonymous" src="https://e.bientot.dev/static/array.js"></script>`);
    expect(page).toContain(`<script defer src="/analytics.js"></script>`);
    expect(page).not.toContain("/_vercel/");
  });

  // The domain, the repository and the theme keep the old name; the pages say it only to tell
  // that the app was renamed.
  test.each(["/", "/statistiques"])("%p names the app Bientôt", async (path) => {
    const page = await (await get(path))!.text();
    expect(page).toContain('<a class="brand" href="/">');
    expect(page).toContain('<meta property="og:site_name" content="Bientôt">');
    expect(page).toContain("L’application s’appelait WorkTime jusqu’en octobre 2026.");
    const rest = page
      .replace(/https:\/\/[^"\s]+/g, "")
      .replace("L’application s’appelait WorkTime jusqu’en octobre 2026.", "")
      .replace('"alternateName":"WorkTime"', "")
      .replace("Thanaen/Bientot</a>", "")
      .replace("Thème WorkTime One", "");
    expect(rest).not.toContain("WorkTime");
  });

  test("every local file the page links to is served", async () => {
    const paths = new Set(
      [...(await html()).matchAll(/(?:(?:href|src|content)="|url\(")(?:https:\/\/bientot\.dev)?(\/[\w./-]+\.\w+)"/g)].map(
        (match) => match[1] ?? "",
      ),
    );
    expect(paths).toContain("/og/home.png");
    expect([...paths].filter((path) => path.startsWith("/fonts/")).sort()).toEqual(Object.keys(FONTS).sort());
    expect(paths).toContain("/analytics.js");
    for (const path of paths) {
      const response = await get(path);
      expect(response?.status, path).toBe(200);
      expect((await response!.arrayBuffer()).byteLength, path).toBeGreaterThan(100);
    }
  });

  // A font file's name carries a hash of its content (scripts/fonts/subset.py).
  test("font files may be kept for good", async () => {
    for (const path of Object.keys(FONTS)) {
      expect(path).toMatch(/^\/fonts\/(sans|mono)-\d00-(core|ext)\.[0-9a-f]{8}\.woff2$/);
      const response = (await get(path))!;
      expect(response.headers.get("content-type")).toBe("font/woff2");
      expect(response.headers.get("cache-control")).toBe("public, max-age=31536000, immutable");
    }
  });

  test("the share image is a 1200 × 630 PNG", async () => {
    const bytes = new DataView(await (await get("/og/home.png"))!.arrayBuffer());
    expect(bytes.getUint32(16)).toBe(1200);
    expect(bytes.getUint32(20)).toBe(630);
  });

  test("robots.txt points at the sitemap, which lists the page", async () => {
    expect(await (await get("/robots.txt"))!.text()).toContain("Sitemap: https://bientot.dev/sitemap.xml");
    expect(await (await get("/sitemap.xml"))!.text()).toContain("<loc>https://bientot.dev/</loc>");
  });

  test.each(["/fonts/OFL.txt", "/og/home.html", "/fonts/IBMPlexSans-Regular.ttf", "/fonts/IBMPlexSans-Regular.woff2"])("serves no file at %p", async (path) => {
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
    const routes: Record<string, { GET(request: Request): Response | Promise<Response> }> = app as never;
    const get = (path: string) => routes[path]!.GET(new Request(`http://local${path}`));
    expect(await (await get("/")).text()).toContain('href="/statistiques"');
    expect(await (await get("/sitemap.xml")).text()).toContain("<loc>https://bientot.dev/statistiques</loc>");
  });

  test("the page carries the site's head", async () => {
    const html = await (await page()).text();
    expect(html).toContain("<title>Statistiques · Bientôt</title>");
    expect(html).toContain('<link rel="canonical" href="https://bientot.dev/statistiques">');
    expect(html).toContain(`<script defer src="/analytics.js"></script>`);
    expect(html).not.toContain("<!-- head -->");
  });
});

// Through Bun.serve, with the Host header Vercel passes for each of the project's domains.
describe("the two domains", () => {
  let server: ReturnType<typeof Bun.serve>;
  beforeEach(() => {
    server = Bun.serve({ port: 0, routes: app, fetch: () => new Response(null, { status: 404 }) });
  });
  afterEach(() => server.stop(true));
  const call = (host: string, path: string, init: RequestInit = {}) =>
    fetch(new URL(path, server.url), { ...init, redirect: "manual", headers: { host } });
  const signal = JSON.stringify({ version: "0.2.0-alpha.37", channel: "direct", first_this_month: true });

  // What installed copies, old builds included, and other sites call. The usage signal follows no
  // redirect, so all of these answer on the old domain exactly as on the new one.
  test.each([
    ["POST", "/v1/ping"],
    ["GET", "/updates/releases.win.json"],
    ["GET", "/updates/Thanaen.WorkTime-0.2.0-alpha.37-full.nupkg"],
    ["GET", "/updates/Thanaen.WorkTime-win-Portable.zip"],
    ["GET", "/updates/RELEASES"],
    ["GET", "/download"],
    ["GET", "/statistiques/mensuel.csv"],
    ["GET", "/robots.txt"],
    ["GET", "/sitemap.xml"],
    ["GET", "/og/home.png"],
    ["GET", Object.keys(FONTS)[0]!],
    ["GET", "/analytics.js"],
    ["GET", "/favicon.ico"],
  ])("%s %s answers on worktime.thanaen.dev as on bientot.dev", async (method, path) => {
    const init = method === "POST" ? { method, body: signal } : { method };
    const before = await call("worktime.thanaen.dev", path, init);
    const after = await call("bientot.dev", path, init);
    expect(before.status).not.toBe(301);
    expect(before.status).toBe(after.status);
    expect(before.headers.get("location")).toBe(after.headers.get("location"));
    expect(Buffer.from(await before.arrayBuffer()).equals(Buffer.from(await after.arrayBuffer()))).toBe(true);
  });

  test("the old domain records the signal without a redirect", async () => {
    const response = await call("worktime.thanaen.dev", "/v1/ping", { method: "POST", body: signal });
    expect(response.status).toBe(204);
    expect(response.headers.get("location")).toBeNull();
  });

  test("the old domain still points the feed and the installer at the GitHub releases", async () => {
    const releases = "https://github.com/Thanaen/Bientot/releases/latest/download";
    const feed = await call("worktime.thanaen.dev", "/updates/releases.win.json");
    expect(feed.status).toBe(302);
    expect(feed.headers.get("location")).toBe(`${releases}/releases.win.json`);
    const setup = await call("worktime.thanaen.dev", "/download");
    expect(setup.status).toBe(302);
    expect(setup.headers.get("location")).toBe(`${releases}/Bientot-Setup.exe`);
  });

  test.each([
    ["/", "https://bientot.dev/"],
    ["/statistiques", "https://bientot.dev/statistiques"],
    ["/?utm_source=x", "https://bientot.dev/?utm_source=x"],
  ])("the old domain moves the page %p to bientot.dev", async (path, location) => {
    const response = await call("worktime.thanaen.dev", path);
    expect(response.status).toBe(301);
    expect(response.headers.get("location")).toBe(location);
  });

  test.each([
    ["/", "https://bientot.dev/"],
    ["/statistiques", "https://bientot.dev/statistiques"],
  ])("bientot.dev serves the page %p", async (path, canonical) => {
    const response = await call("bientot.dev", path);
    expect(response.status).toBe(200);
    expect(await response.text()).toContain(`<link rel="canonical" href="${canonical}">`);
  });
});
