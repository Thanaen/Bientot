# WorkTime service

The one web service of WorkTime, at `https://worktime.thanaen.dev`. It shows the homepage,
counts the active installations without identifying anyone, and points installed copies at
their updates.

| Route | Answer |
| --- | --- |
| `GET /` | The homepage, `site/index.html` |
| `GET /fonts/<file>`, `/og/<page>.png`, `/favicon.*`, `/apple-touch-icon.png` | The files listed in `src/site.ts`; `404` for any other path |
| `GET /robots.txt`, `/sitemap.xml` | Generated from the pages listed in `src/site.ts` |
| `POST /v1/ping` | `204`; `400` for any other shape, `413` above 256 bytes |
| `GET /v1/stats` | The last 12 calendar months, with `Authorization: Bearer $STATS_TOKEN` |
| `GET /updates/<file>` | `302` to that file of the Velopack release in the Blob store; `404` for a name vpk never writes |
| `GET /download` | `302` to the latest `Setup.exe` |

## Pages

`site/index.html` is a static page in French, in the app's WorkTime One theme. It runs no
script, sets no cookie and makes no third-party request: the fonts are Latin subsets of the
app's IBM Plex fonts, made with `pyftsubset`.

`src/site.ts` lists the pages and the files they use. Each page has `<!-- head -->` in its
`<head>`; the service fills it with the title, description, canonical URL, link-preview tags
(Open Graph, `summary_large_image`), icons and theme colours, and adds the page to
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

`channel` is `store`, `direct` (installed by `Setup.exe`) or `zip`. The service keeps one
counter per UTC day, version and channel in Turso. It stores no address, no identifier and no
request body, and logs nothing.

`activeThisMonth` counts first-of-month signals. `averageDaily` and
`peakDaily` count every signal of a day; the average, to one decimal, spreads them over every
day of the month, or every day so far for the current month.

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
