# WorkTime service

The one web service of WorkTime, at `https://worktime.thanaen.dev`. It shows the homepage,
counts the active installations without identifying anyone, and points installed copies at
their updates.

| Route | Answer |
| --- | --- |
| `GET /` | The homepage, `site/index.html` |
| `GET /statistiques` | The public statistics page, built from `site/statistiques.html` |
| `GET /statistiques/<name>.<csv\|json>` | The open data files: `mensuel`, `quotidien`, `details` |
| `GET /fonts/<file>` | One of the IBM Plex fonts in `site/fonts/` (SIL OFL); `404` for any other name |
| `POST /v1/ping` | `204`; `400` for any other shape, `413` above 256 bytes |
| `GET /v1/stats` | The last 12 calendar months, with `Authorization: Bearer $STATS_TOKEN` |
| `GET /updates/<file>` | `302` to that file of the Velopack release in the Blob store; `404` for a name vpk never writes |
| `GET /download` | `302` to the latest `Setup.exe` |

## Homepage

`site/index.html` is a static page in French, in the app's WorkTime One theme. It uses no
script, no cookie and no third-party request: the fonts are Latin subsets of the app's IBM
Plex fonts, made with `pyftsubset`. A new font must also be listed in `src/site.ts`.

## Usage signal

Once a day, WorkTime sends:

```json
{"version":"1.0.0","channel":"store","first_this_month":false}
```

`channel` is `store`, `direct` (installed by `Setup.exe`) or `zip`. The service keeps one
counter per UTC day, version and channel in Turso. It stores no address, no identifier and no
request body, and logs nothing.

`activeThisMonth` counts first-of-month signals. `averageDaily` and
`peakDaily` count every signal of a day; the average, to one decimal, spreads them over every
day of the month, or every day so far for the current month.

## Statistics

`/statistiques` publishes what the counters allow, as a page and as CSV and JSON files under CC0.
`src/stats.ts` holds the publication rules, tested in `stats.test.ts`:

- only finished UTC days: the current day appears the next day;
- no figure under 10: the cell is empty (`null`), or a version joins `autres`;
- the version and channel figures are rounded to the nearest 5, so that a hidden cell cannot be
  found by subtracting the others from the month's exact total;
- the last 12 calendar months.

The figures change once a day, so they cost almost nothing to serve. On a cache miss the function
runs two `GROUP BY` queries over the primary key, a few hundred rows, and keeps the result in memory
until the day changes. Each answer carries `Cache-Control: public, max-age=600` for browsers,
`Vercel-CDN-Cache-Control` up to 00:05 UTC the next day with an hour of `stale-while-revalidate`,
and an `ETag` named after the last day it covers. The page is drawn on the server as HTML and CSS,
with no script.

## Updates

`scripts/release-direct.sh` (repository root) builds the release: `Setup.exe`, the packages and
the `releases.win.json` feed. `scripts/publish-release.ts` uploads that folder to the
`worktime-releases` Blob store under `updates/`, packages first and the feed last:

```sh
BLOB_READ_WRITE_TOKEN=… bun scripts/publish-release.ts <output-dir>
```

The app reads `https://worktime.thanaen.dev/updates/releases.win.json` with no query string and
no identifier. The redirects are not cached; the feed, the installer and the portable ZIP are
cached for a minute at most in the store.

## Run

```sh
bun install
bun test
bun run dev        # http://localhost:3000, data in ./usage-ping.db
```

## Deploy

Vercel project `worktime`, root directory `services/worktime`, Bun runtime (`vercel.json`),
function region `dub1` (a project setting, next to the Turso database), domain
`worktime.thanaen.dev`. Variables: `TURSO_DATABASE_URL`,
`TURSO_AUTH_TOKEN`, `STATS_TOKEN`, and `RELEASES_URL`, the public URL of the
`worktime-releases` Blob store (`https://<store>.public.blob.vercel-storage.com`), which
publish-release.ts prints. The table is created on the first request.

The `worktime-releases` store is not connected to the project, and its read-write token stays
with whoever publishes: the packages are unsigned, so whoever can write to the store chooses
what every installed copy runs at its next start.

The project is connected to GitHub, with `main` as the production branch: a push to `main`
deploys to production, and a push to another branch builds a preview. The project's Ignored
Build Step, `git diff --quiet "${VERCEL_GIT_PREVIOUS_SHA:-HEAD^}" HEAD -- .`, skips a commit
when nothing in this folder changed since the branch's last deployment, so a push that only
touches files outside this folder neither builds nor turns a pull request red. On a branch's
first deployment there is no previous deployment, so it compares with the parent commit only.
Builds run on Vercel and use no GitHub Actions minutes.

To redeploy without a commit, run `vercel deploy --prod` from the repository root.
