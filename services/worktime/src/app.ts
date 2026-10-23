import { MAX_BODY_BYTES, parsePing, utcDay } from "./ping";
import { SETUP, redirect, releaseUrl } from "./releases";
import { font, homepage } from "./site";
import { DATASETS, type Dataset, type PublicStats, firstDay, lastFinishedDay, publish, toCsv } from "./stats";
import { renderPage } from "./stats-page";
import type { UsageStore } from "./store";

export interface AppOptions {
  store: UsageStore;
  statsToken: string | undefined;
  // The public URL of the Blob store that holds the releases; without it, they are not found.
  releasesUrl?: string;
  now?: () => Date;
}

// Nothing here logs the request, its body or the caller's address.
export function createApp({ store, statsToken, releasesUrl, now = () => new Date() }: AppOptions) {
  // The public figures only change when a UTC day ends: a warm function computes them once per day,
  // and the CDN keeps each answer until the next day (statsHeaders), so the database is rarely read.
  let cached: { through: string; stats: Promise<PublicStats> } | undefined;
  const publicStats = () => {
    const through = lastFinishedDay(now());
    if (cached?.through !== through) {
      const stats = store.totals(firstDay(through), through).then(({ days, cells }) => publish(days, cells, through));
      stats.catch(() => (cached = undefined));
      cached = { through, stats };
    }
    return cached.stats;
  };
  const statsResponse = async (request: Request, name: string, type: string, body: (stats: PublicStats) => string | Promise<string>) => {
    const stats = await publicStats();
    const headers = statsHeaders(now(), `"${name}-${stats.through}"`, type);
    // Vercel compresses the answer and turns the ETag into a weak one (W/"…"), which browsers send back.
    const known = (request.headers.get("if-none-match") ?? "").split(",").map((tag) => tag.trim().replace(/^W\//, ""));
    if (known.includes(headers.etag)) return new Response(null, { status: 304, headers });
    return new Response(await body(stats), { headers });
  };

  return {
    "/": {
      GET: homepage,
    },
    "/fonts/:file": {
      GET(request: Request & { params: { file: string } }): Response {
        return font(request.params.file);
      },
    },
    "/v1/ping": {
      async POST(request: Request): Promise<Response> {
        const text = await request.text();
        if (new TextEncoder().encode(text).length > MAX_BODY_BYTES) return new Response(null, { status: 413 });

        let body: unknown;
        try {
          body = JSON.parse(text);
        } catch {
          return new Response(null, { status: 400 });
        }
        const ping = parsePing(body);
        if (!ping) return new Response(null, { status: 400 });

        await store.record(ping, utcDay(now()));
        return new Response(null, { status: 204 });
      },
    },
    "/v1/stats": {
      async GET(request: Request): Promise<Response> {
        if (!statsToken || request.headers.get("authorization") !== `Bearer ${statsToken}`) {
          return new Response(null, { status: 401 });
        }
        return Response.json(await store.months(now(), 12), { headers: { "cache-control": "no-store" } });
      },
    },
    "/statistiques": {
      GET(request: Request): Promise<Response> {
        return statsResponse(request, "page", "text/html; charset=utf-8", renderPage);
      },
    },
    "/statistiques/:file": {
      async GET(request: Request & { params: { file: string } }): Promise<Response> {
        const match = /^(mensuel|quotidien|details)\.(csv|json)$/.exec(request.params.file);
        if (!match) return new Response(null, { status: 404 });
        const dataset = DATASETS[match[1] as Dataset];
        const csv = match[2] === "csv";
        const response = await statsResponse(
          request,
          request.params.file,
          csv ? "text/csv; charset=utf-8" : "application/json",
          (stats) => (csv ? toCsv(dataset.columns, dataset.rows(stats)) : JSON.stringify(dataset.rows(stats))),
        );
        // Open data: scripts on other sites may read the files.
        response.headers.set("access-control-allow-origin", "*");
        return response;
      },
    },
    "/updates/:file": {
      GET(request: Request & { params: { file: string } }): Response {
        return redirect(releasesUrl ? releaseUrl(releasesUrl, request.params.file) : null);
      },
    },
    "/download": {
      GET(): Response {
        return redirect(releasesUrl ? releaseUrl(releasesUrl, SETUP) : null);
      },
    },
  };
}

// Browsers keep an answer ten minutes; Vercel's CDN keeps it until five minutes past the next UTC
// midnight, then serves it for one more hour while it fetches the new day's figures.
function statsHeaders(now: Date, etag: string, type: string): Record<string, string> & { etag: string } {
  const refresh = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() + 1, 0, 5);
  const seconds = Math.max(60, Math.round((refresh - now.getTime()) / 1000));
  return {
    "content-type": type,
    "cache-control": "public, max-age=600",
    "vercel-cdn-cache-control": `max-age=${seconds}, stale-while-revalidate=3600`,
    etag,
  };
}
