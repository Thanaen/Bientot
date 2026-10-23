import { createApp } from "./src/app";
import { MemoryStore, PostHogStore } from "./src/store";

// On Vercel, the WorkTime project in PostHog EU; its token only lets one send events, so it is no
// secret. Under `bun run dev`, the signals stay in memory.
const store = process.env.VERCEL
  ? new PostHogStore({
      projectToken: "phc_omakaFDpSAzS5xcHqCbt8fhatcbUF2xgQhz6Tixi872Q",
      projectId: "317586",
      apiKey: process.env.POSTHOG_PERSONAL_API_KEY,
    })
  : new MemoryStore();

Bun.serve({
  routes: createApp({ store, releasesUrl: process.env.RELEASES_URL }),
  fetch() {
    return new Response(null, { status: 404 });
  },
});
