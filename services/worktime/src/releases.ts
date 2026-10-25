// The Velopack releases of Bientôt, published as GitHub Releases by the Release workflow
// (.github/workflows/release.yml). The installed app reads its feed at /updates/ (FEED_URL in
// crates/worktime-app/src/updates/direct.rs), and /download serves the installer: both stay at
// this address and redirect to the release files, wherever those live.

const RELEASES = "https://github.com/Thanaen/WorkTime/releases";

export const SETUP = "Thanaen.WorkTime-win-Setup.exe";

// The packages vpk names after their version, which names their release: v<version>.
const PACKAGE = /^Thanaen\.WorkTime-(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?)-(?:full|delta)\.nupkg$/;

// The files of which only the latest release's copy matters: its feed lists every package.
const LATEST = new Set(["releases.win.json", SETUP, "Thanaen.WorkTime-win-Portable.zip"]);

// Where `file` is published, or null for a name the Release workflow never publishes.
export function releaseUrl(file: string): string | null {
  const version = PACKAGE.exec(file)?.[1];
  if (version) return `${RELEASES}/download/v${version}/${file}`;
  if (LATEST.has(file)) return `${RELEASES}/latest/download/${file}`;
  return null;
}

// A redirect nobody caches, so a new feed is read as soon as it is published.
export function redirect(location: string | null): Response {
  if (!location) return new Response(null, { status: 404 });
  return new Response(null, { status: 302, headers: { location, "cache-control": "no-store" } });
}
