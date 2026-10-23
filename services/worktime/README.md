# WorkTime service

The one web service of WorkTime, at `https://worktime.thanaen.dev`. It shows the homepage,
counts the active installations without identifying anyone, and points installed copies at
their updates.

| Route | Answer |
| --- | --- |
| `GET /` | The homepage, `site/index.html` |
| `GET /statistiques` | The public statistics page, built from `site/statistiques.html` |
| `GET /statistiques/<name>.<csv\|json>` | The open data files: `mensuel`, `quotidien`, `details` |
| `GET /fonts/<file>`, `/og/<page>.png`, `/favicon.*`, `/apple-touch-icon.png`, `/analytics.js` | The files listed in `src/site.ts`; `404` for any other path |
| `GET /robots.txt`, `/sitemap.xml` | Generated from the pages listed in `src/site.ts` |
| `POST /v1/ping` | `204`; `400` for any other shape, `413` above 256 bytes |
| `GET /updates/<file>` | `302` to that file in the GitHub Releases (see Updates); `404` for a name the Release workflow never publishes |
| `GET /download` | `302` to the latest `Setup.exe` |

## Pages

`site/index.html` is a static page in French, in the app's WorkTime One theme. It sets no
cookie: the fonts are Latin subsets of the app's IBM Plex fonts, made with `pyftsubset`. Its
only scripts measure the audience and the loading speed with PostHog (see `docs/privacy.md`):
PostHog's library from `https://eu-assets.i.posthog.com`, then `site/analytics.js`, which starts
it in cookieless mode with Web Vitals and nothing else (no autocapture, no session recording, no
surveys, no feature flags), sending to `https://eu.i.posthog.com`. A managed reverse proxy,
`e.worktime.thanaen.dev` (a CNAME on Cloudflare, DNS only), is set up but PostHog has not issued
its certificate yet; once it serves, both addresses can move to it.

`src/site.ts` lists the pages and the files they use. Each page has `<!-- head -->` in its
`<head>`; the service fills it with the title, description, canonical URL, link-preview tags
(Open Graph, `summary_large_image`), icons, theme colours and the two analytics scripts, and adds the page to
`/sitemap.xml`. A new page or file must be listed there.

Each page has a 1200 × 630 share image, drawn from an HTML source next to it. After changing
`site/og/home.html`, regenerate the PNG:

```sh
playwright screenshot --viewport-size=1200,630 "file://$PWD/site/og/home.html" site/og/home.png
```

## Usage signal

Once a day, WorkTime sends:

```json
{"version":"1.0.0","channel":"store","first_this_month":false}
```

`channel` is `store`, `direct` (installed by `Setup.exe`) or `zip`. The service checks the shape,
then sends the signal to the WorkTime project in PostHog EU as a `usage_ping` event
(`src/store.ts`): the three fields, a random id used once, no person profile
(`$process_person_profile: false`), no IP address and no GeoIP lookup, and the UTC day's midnight
as its time, so that the signals of one day cannot be told apart. The project also discards IP
addresses. The service stores nothing and logs nothing. When PostHog refuses the event, the
service answers `500` and the app tries again within the hour.

## Statistics

`/statistiques` publishes what the counters allow, as a page and as CSV and JSON files under CC0.
`src/stats.ts` holds the publication rules, tested in `stats.test.ts`:

- only finished UTC days: the current day appears the next day;
- monthly figures only for finished months, so that two snapshots of a growing total never show
  what one day added;
- no figure under 10: the cell is empty (`null`), or the key joins `autres`. While `autres` is
  under 10, the smallest shown key joins it too, so that no hidden figure can be found by
  subtracting the shown ones from the month's exact total;
- the version and channel figures are rounded to the nearest 5;
- a month's daily average and peak come from the published days, a hidden day counting as 0;
- a month whose actives exceed the total of its published days stays hidden: every first-of-month
  signal is also a day's signal, so the excess would tell what the hidden days hold;
- the last 12 finished months.

The figures come from the `usage_daily` endpoint of the PostHog project, a saved SQL query
pinned to its version in `src/store.ts` (`ENDPOINT`). It answers one row per UTC day, version and
channel over the last 400 days: the signals, and the first-of-month signals among them. The
service asks for a fresh answer (`refresh: "force"`), since a cached one could predate the end of
the day it publishes; it does so once per finished day and instance. The service calls it with a
project secret API key limited to `endpoint:read`, `POSTHOG_PERSONAL_API_KEY`; without it, the page and the files answer `500`.

```sql
SELECT
  toString(toDate(timestamp)) AS day,
  toString(properties.version) AS version,
  toString(properties.channel) AS channel,
  count() AS pings,
  countIf(properties.first_this_month = true) AS first_of_month
FROM events
WHERE event = 'usage_ping'
  AND timestamp >= now() - INTERVAL 400 DAY
GROUP BY day, version, channel
ORDER BY day, version, channel
LIMIT 10000
```

The figures change once a day, so they cost almost nothing to serve. On a cache miss the function
calls the endpoint once and keeps the result in memory until the day changes. Each answer carries `Cache-Control: public, max-age=600` for browsers,
`Vercel-CDN-Cache-Control` up to 00:05 UTC the next day with an hour of `stale-while-revalidate`,
and an `ETag` named after the last day it covers. The page is drawn on the server as HTML and CSS,
with no script.

## Updates

The Release workflow (`.github/workflows/release.yml`) publishes each release as a GitHub Release
named `v<version>`: `Setup.exe`, the packages and the `releases.win.json` feed, which lists every
package. The app reads `https://worktime.thanaen.dev/updates/releases.win.json` with no query
string and no identifier. The service redirects, without caching:

- the feed, `Setup.exe` (also at `/download`) and the portable ZIP to the latest release's copy;
- a package, `Thanaen.WorkTime-<version>-full.nupkg` or `-delta.nupkg`, to the release of its
  version.

These addresses stay the same wherever the files live.

## Run

```sh
bun install
bun test
bun run dev        # http://localhost:3000, signals kept in memory
```

## Deploy

Vercel project `worktime`, root directory `services/worktime`, Bun runtime (`vercel.json`),
function region `dub1` (a project setting), domain `worktime.thanaen.dev`. Variable:
`POSTHOG_PERSONAL_API_KEY` (above). The PostHog project token and id are in `server.ts`: the token only
lets one send events. Outside Vercel, the signals stay in memory.

The packages are unsigned, so whoever can publish a release chooses what every installed copy
runs at its next start: only the Release workflow's publish job can write to the repository's
releases.

The project is connected to GitHub, with `main` as the production branch: a push to `main`
deploys to production, and a push to another branch builds a preview. The `ignoreCommand` in
`vercel.json` skips a commit when nothing in this folder changed since the branch's last
deployment, so a push that only touches files outside this folder neither builds nor turns a
pull request red. On a branch's first deployment there is no previous deployment, so it
compares with the parent commit. When that commit is gone, after a squash merge or a force
push, it builds instead of failing.
Builds run on Vercel and use no GitHub Actions minutes.

To redeploy without a commit, run `vercel deploy --prod` from the repository root.
