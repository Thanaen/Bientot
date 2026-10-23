import { MAX_BODY_BYTES, parsePing, utcDay } from "./ping";
import { SETUP, redirect, releaseUrl } from "./releases";
import { siteRoutes } from "./site";
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
  return {
    ...siteRoutes(),
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
