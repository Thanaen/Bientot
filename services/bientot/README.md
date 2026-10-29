# Bientôt service

The one web service of Bientôt, at `https://bientot.dev`. It shows the homepage,
counts the active installations without identifying anyone, and points installed copies at
their updates.

The service also answers at `https://worktime.thanaen.dev`, the site's first address. Installed
copies read their update feed and send their usage signal there (`FEED_URL` in
`crates/bientot-app/src/updates/direct.rs`, `ENDPOINT` in `crates/bientot-core/src/usage_ping.rs`),
and the usage signal follows no redirect. On that host, every route answers exactly as on
`bientot.dev`, except the two HTML pages, `/` and `/statistiques`, which answer `301` to the same
address on `bientot.dev` (`moved` in `src/site.ts`, decided by the `Host` header). The app's two
addresses move to `bientot.dev` in a later release; the old host must keep serving until the
statistics show no older version.

| Route | Answer |
| --- | --- |
| `GET /` | The homepage, `site/index.html`; `301` to `bientot.dev` on the old host |
| `GET /statistiques` | The public statistics page, built from `site/statistiques.html`; `301` to `bientot.dev` on the old host |
| `GET /statistiques/<name>.<csv\|json>` | The open data files: `mensuel`, `quotidien`, `details` |
| `GET /fonts/<file>`, `/og/<page>.png`, `/favicon.*`, `/apple-touch-icon.png`, `/analytics.js` | The files listed in `src/site.ts`; `404` for any other path |
| `GET /robots.txt`, `/sitemap.xml` | Generated from the pages listed in `src/site.ts` |
| `POST /v1/ping` | `204`; `400` for any other shape, `413` above 256 bytes |
| `GET /updates/<file>` | `302` to that file in the GitHub Releases (see Updates); `404` for a name the Release workflow never publishes |
| `GET /download` | `302` to the latest installer, `Bientot-Setup.exe` (see Updates) |

## Pages

`site/index.html` is a static page in French, in the app's WorkTime One theme. It sets no
cookie: the fonts are subsets of the app's IBM Plex fonts (see Fonts below). Its
only scripts measure the audience and the loading speed with PostHog (see `docs/privacy.md`):
PostHog's library from `https://e.bientot.dev`, then `site/analytics.js`, which starts
it in cookieless mode with Web Vitals and nothing else (no autocapture, no session recording, no
surveys, no feature flags). `e.bientot.dev` is PostHog's managed reverse proxy for the
Bientôt project (a CNAME in the Vercel DNS of `bientot.dev`), so ad blockers that block PostHog's
own domains still let the page load. The older proxy `e.worktime.thanaen.dev` (a CNAME on
Cloudflare) still answers for pages cached before the switch.

`src/site.ts` lists the pages and the files they use. Each page has `<!-- head -->` in its
`<head>`; the service fills it with the title, description, canonical URL on `bientot.dev`, link-preview tags
(Open Graph, `summary_large_image`), icons, theme colours and the two analytics scripts, and adds the page to
`/sitemap.xml`. A new page or file must be listed there.

Each page has a 1200 × 630 share image, drawn from an HTML source next to it. After changing
`site/og/home.html`, regenerate the PNG:

```sh
playwright screenshot --viewport-size=1200,630 "file://$PWD/site/og/home.html" site/og/home.png
```

### Fonts

The pages use five faces of the app's IBM Plex fonts (`crates/bientot-app/assets/fonts`): Sans
400, 500 and 600, Mono 400 and 500. `scripts/fonts/subset.sh` cuts each face into two WOFF2 files
in `site/fonts`:

- a core file with printable ASCII, the no-break space, curly quotes, guillemets, en and em dashes,
  the ellipsis and every other character its family draws on the pages. Each page preloads the
  core files of the faces it uses.
- an extended file with the rest of Latin-1 and Latin Extended-A. Its `unicode-range` keeps it from
  downloading until a page draws one of its characters; no page does today.

Each file keeps the source's outlines, metrics and TrueType hinting, and only the OpenType features
its text uses (`kern`, and `liga` for « fi »). Plex is a Reserved Font Name under the OFL
(`site/fonts/OFL.txt`), so the subsets are named Bientot Sans and Bientot Mono inside the files. The
file names carry a hash of their content, so the service lets browsers keep them for a year.

The script renders every page and state with the service's code (`scripts/fonts/pages.ts`), reads in
Chromium which characters each face draws (`scripts/fonts/browse.ts`), then writes the fonts, their
`@font-face` rules and preloads in `site/index.html`, `site/statistiques.html` and
`site/og/home.html`, and `src/fonts.ts`. It is not part of the build: run it by hand and commit what
it writes. It needs bun, Playwright's Chromium, python3 and `pip install fonttools brotli`.

```sh
./scripts/fonts/subset.sh
```

Run it again after a change that adds characters to the pages or the statistics, changes a
`font-weight`, `font-family`, `text-overflow` or `hyphens`, or adds a face. Until then, a new
character outside the core files still shows in Plex, from the extended file, a moment later;
one outside Latin-1 and Latin Extended-A shows in the fallback font.

To check that a change leaves the pages looking the same, render the pages of both versions and
compare their screenshots (`--hinting full` applies the fonts' hinting, as Windows does):

```sh
git worktree add /tmp/before main
bun scripts/fonts/pages.ts /tmp/before/services/bientot /tmp/pages-before
bun scripts/fonts/browse.ts /tmp/pages-before /tmp/before/services/bientot /tmp/before.json --shots /tmp/shots-before --hinting full
bun scripts/fonts/pages.ts /tmp/pages-after
bun scripts/fonts/browse.ts /tmp/pages-after . /tmp/after.json --shots /tmp/shots-after --hinting full
python3 scripts/fonts/diff.py /tmp/shots-before /tmp/shots-after /tmp/shots-diff
```

The reports list the font files each page downloads and preloads. `specimen.html` is not a page of
the site: it shows text the pages do not use today (italics, other Latin letters, CJK).

## Usage signal

Once a day, Bientôt sends:

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
named `v<version>`: the installer, the portable ZIP, the packages and the `releases.win.json`
feed, which lists every package. vpk names the files after the packId, `Thanaen.WorkTime`, which
installed copies update by: `Thanaen.WorkTime-win-Setup.exe`, `Thanaen.WorkTime-win-Portable.zip`
and `Thanaen.WorkTime-<version>-full.nupkg` or `-delta.nupkg`. The workflow also publishes the
installer and the ZIP as `Bientot-Setup.exe` and `Bientot-Portable.zip`. The app reads
`https://worktime.thanaen.dev/updates/releases.win.json` with no query string and no identifier.
The service redirects, without caching:

- the feed, the installer and the portable ZIP, under either name, to the latest release's copy;
- a package to the release of its version;
- `/download` to `Bientot-Setup.exe`.

These addresses stay the same wherever the files live.

## Run

```sh
bun install
bun test
bun run dev        # http://localhost:3000, signals kept in memory
```

## Deploy

Vercel project `worktime`, root directory `services/bientot`, Bun runtime (`vercel.json`),
function region `dub1` (a project setting), domains `bientot.dev` (with `www.bientot.dev`
redirecting to it) and `worktime.thanaen.dev`. Variable:
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
