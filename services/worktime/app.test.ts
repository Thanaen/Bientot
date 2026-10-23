import { createClient } from "@libsql/client";
import { beforeEach, describe, expect, test } from "bun:test";
import { createApp } from "./src/app";
import { UsageStore } from "./src/store";

let clock: Date;
let app: ReturnType<typeof createApp>;

beforeEach(() => {
  clock = new Date("2026-10-12T22:30:00Z");
  app = createApp({ store: new UsageStore(createClient({ url: ":memory:" })), statsToken: "secret", now: () => clock });
});

const ping = (body: unknown) =>
  app["/v1/ping"].POST(
    new Request("http://local/v1/ping", { method: "POST", body: typeof body === "string" ? body : JSON.stringify(body) }),
  );
const stats = async (token = "secret") => {
  const response = await app["/v1/stats"].GET(
    new Request("http://local/v1/stats", { headers: { authorization: `Bearer ${token}` } }),
  );
  return { status: response.status, body: response.status === 200 ? await response.json() : null };
};

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

describe("GET /v1/stats", () => {
  test("needs the stats token", async () => {
    expect((await stats("wrong")).status).toBe(401);
  });

  test("counts monthly actives from first-of-month signals and daily actives from every signal", async () => {
    await ping({ version: "1.0.0", channel: "store", first_this_month: true });
    await ping({ version: "1.0.0", channel: "store", first_this_month: true });
    await ping({ version: "1.0.0-alpha.3", channel: "zip", first_this_month: true });
    clock = new Date("2026-10-13T08:00:00Z");
    await ping({ version: "1.0.0", channel: "store", first_this_month: false });
    clock = new Date("2026-11-01T08:00:00Z");
    await ping({ version: "1.0.1", channel: "store", first_this_month: true });

    expect((await stats()).body).toEqual([
      { month: "2026-11", activeThisMonth: 1, averageDaily: 1, peakDaily: 1, byChannel: { store: 1 }, byVersion: { "1.0.1": 1 } },
      {
        month: "2026-10",
        activeThisMonth: 3,
        averageDaily: 0.1,
        peakDaily: 3,
        byChannel: { store: 2, zip: 1 },
        byVersion: { "1.0.0": 2, "1.0.0-alpha.3": 1 },
      },
    ]);
  });

  test("averages a finished month over all its days, quiet ones included", async () => {
    clock = new Date("2026-09-01T09:00:00Z");
    for (let i = 0; i < 30; i++) await ping({ version: "1.0.0", channel: "store", first_this_month: true });
    clock = new Date("2026-10-12T09:00:00Z");

    const [september] = (await stats()).body;
    expect(september).toMatchObject({ month: "2026-09", averageDaily: 1, peakDaily: 30 });
  });

  test("averages the current month over the days elapsed so far", async () => {
    clock = new Date("2026-10-01T09:00:00Z");
    for (let i = 0; i < 4; i++) await ping({ version: "1.0.0", channel: "store", first_this_month: true });
    clock = new Date("2026-10-02T09:00:00Z");

    expect((await stats()).body).toEqual([expect.objectContaining({ month: "2026-10", averageDaily: 2, peakDaily: 4 })]);
  });

  test("keeps the last 12 calendar months, even when recent months are quiet", async () => {
    clock = new Date("2024-01-15T09:00:00Z");
    await ping({ version: "0.9.0", channel: "zip", first_this_month: true });
    clock = new Date("2025-11-01T09:00:00Z");
    await ping({ version: "0.9.5", channel: "zip", first_this_month: true });
    clock = new Date("2026-10-12T09:00:00Z");

    expect((await stats()).body.map((m: { month: string }) => m.month)).toEqual(["2025-11"]);
  });
});

describe("GET /updates/:file", () => {
  const releases = createApp({
    store: new UsageStore(createClient({ url: ":memory:" })),
    statsToken: "secret",
    releasesUrl: "https://store.public.blob.vercel-storage.com/",
  });
  const get = (file: string) =>
    releases["/updates/:file"].GET(Object.assign(new Request(`http://local/updates/${file}`), { params: { file } }));

  test("points at the file in the Blob store, uncached", () => {
    const response = get("releases.win.json");
    expect(response.status).toBe(302);
    expect(response.headers.get("location")).toBe(
      "https://store.public.blob.vercel-storage.com/updates/releases.win.json",
    );
    expect(response.headers.get("cache-control")).toBe("no-store");
  });

  test.each(["..", ".hidden", "a/b", "a%2Fb", ""])("finds no file named %p", (file) => {
    expect(get(file).status).toBe(404);
  });

  test("finds nothing without a store", () => {
    const file = "releases.win.json";
    const response = app["/updates/:file"].GET(
      Object.assign(new Request(`http://local/updates/${file}`), { params: { file } }),
    );
    expect(response.status).toBe(404);
  });

  test("/download points at the installer", () => {
    expect(releases["/download"].GET().headers.get("location")).toBe(
      "https://store.public.blob.vercel-storage.com/updates/Thanaen.WorkTime-win-Setup.exe",
    );
  });
});

describe("the homepage", () => {
  test("/ serves the page, which downloads through /download", async () => {
    const response = app["/"].GET();
    expect(response.headers.get("content-type")).toBe("text/html; charset=utf-8");
    const html = await response.text();
    expect(html).toContain("<title>WorkTime");
    expect(html).toContain('href="/download"');
  });

  test("every font the page asks for is served", async () => {
    const html = await app["/"].GET().text();
    const files = [...html.matchAll(/\/fonts\/([\w-]+\.woff2)/g)].map((match) => match[1] ?? "");
    expect(files.length).toBeGreaterThan(0);
    for (const file of new Set(files)) {
      const response = app["/fonts/:file"].GET(
        Object.assign(new Request(`http://local/fonts/${file}`), { params: { file } }),
      );
      expect(response.headers.get("content-type")).toBe("font/woff2");
      expect((await response.arrayBuffer()).byteLength).toBeGreaterThan(1000);
    }
  });

  test.each(["..", "OFL.txt", "toString", "IBMPlexSans-Regular.ttf"])("serves no font named %p", (file) => {
    const response = app["/fonts/:file"].GET(
      Object.assign(new Request(`http://local/fonts/${file}`), { params: { file } }),
    );
    expect(response.status).toBe(404);
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
    expect(html).toContain("Moins de 10 installations pour l’instant");
    expect(html).not.toContain('class="kpis"');
  });

  test("leaves out the current day and caches until the next one", async () => {
    clock = new Date("2026-10-12T09:00:00Z");
    await pings(12);
    await pings(30, "zip", "0.1.0-alpha.250");
    clock = new Date("2026-10-13T09:00:00Z");
    await pings(15);
    const response = await page();
    expect(response.headers.get("cache-control")).toBe("public, max-age=600");
    // 09:00 to 00:05 the next day.
    expect(response.headers.get("vercel-cdn-cache-control")).toBe("max-age=54300, stale-while-revalidate=3600");
    const html = await response.text();
    expect(html).toContain("Installations actives en octobre");
    expect(html).toContain("Données jusqu’au 12 octobre 2026");
    expect(await (await file("quotidien.csv")).text()).toBe("jour,installations\n2026-10-12,42\n");
  });

  test("serves the open data files, readable from other sites", async () => {
    clock = new Date("2026-10-12T09:00:00Z");
    await pings(12);
    clock = new Date("2026-10-13T09:00:00Z");
    const csv = await file("details.csv");
    expect(csv.headers.get("content-type")).toBe("text/csv; charset=utf-8");
    expect(csv.headers.get("access-control-allow-origin")).toBe("*");
    expect(await csv.text()).toBe("mois,version,mode,actives\n2026-10,0.2.0,direct,10\n");
    expect(await (await file("mensuel.json")).json()).toEqual([
      { mois: "2026-10", actives: 12, moyenne_jour: null, pic_jour: 12, partiel: true },
    ]);
  });

  test("answers 304 to a browser that has the same day's figures", async () => {
    clock = new Date("2026-10-13T09:00:00Z");
    const etag = (await page()).headers.get("etag")!;
    expect((await page({ "if-none-match": etag })).status).toBe(304);
    clock = new Date("2026-10-14T09:00:00Z");
    expect((await page({ "if-none-match": etag })).status).toBe(200);
  });

  test.each(["stats.csv", "mensuel.xml", "..", "mensuel.csv.bak"])("serves no file named %p", async (name) => {
    expect((await file(name)).status).toBe(404);
  });

  test("the homepage links to the page", async () => {
    expect(await app["/"].GET().text()).toContain('href="/statistiques"');
  });
});
