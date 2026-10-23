import { createClient } from "@libsql/client";
import { createApp } from "./src/app";
import { UsageStore } from "./src/store";

const store = new UsageStore(
  createClient({
    url: process.env.TURSO_DATABASE_URL ?? "file:usage-ping.db",
    authToken: process.env.TURSO_AUTH_TOKEN,
  }),
);

Bun.serve({
  routes: createApp({ store, statsToken: process.env.STATS_TOKEN, releasesUrl: process.env.RELEASES_URL }),
  fetch() {
    return new Response(null, { status: 404 });
  },
});
