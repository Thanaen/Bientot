// Tells IndexNow (Bing, and through it DuckDuckGo and the AI assistants that search with it; also
// Yandex, Seznam, Naver) that the pages in the sitemap changed, so they crawl them again soon.
// Run it after a change to the pages reaches production:  bun run scripts/indexnow.ts
import { INDEXNOW_KEY, ORIGIN, PAGES, STATISTICS } from "../src/site";

const response = await fetch("https://api.indexnow.org/indexnow", {
  method: "POST",
  headers: { "content-type": "application/json; charset=utf-8" },
  body: JSON.stringify({
    host: new URL(ORIGIN).host,
    key: INDEXNOW_KEY,
    keyLocation: `${ORIGIN}/${INDEXNOW_KEY}.txt`,
    urlList: [...PAGES, STATISTICS].map((page) => ORIGIN + page.path),
  }),
});
console.log(response.status, response.statusText, await response.text());
// 200 or 202: accepted. 403: the key file is not live yet on ORIGIN.
if (!response.ok) process.exit(1);
