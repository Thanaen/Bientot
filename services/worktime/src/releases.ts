// The Velopack releases of WorkTime, kept in a Vercel Blob store under `updates/` by
// scripts/publish-release.ts. The installed app reads its feed at /updates/ (FEED_URL in
// crates/worktime-app/src/updates/direct.rs); the service only points at the store's public
// files, at RELEASES_URL. It never holds the store's read-write token: whoever writes there
// chooses what every installed copy runs.

// The names vpk gives: releases.win.json, Thanaen.WorkTime-1.0.0-full.nupkg, …-win-Setup.exe.
const FILE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;

export const SETUP = "Thanaen.WorkTime-win-Setup.exe";

// Where `file` lives in the store at `releasesUrl`, or null for a name vpk never writes.
export function releaseUrl(releasesUrl: string, file: string): string | null {
  if (!FILE.test(file) || file.includes("..")) return null;
  return `${releasesUrl.replace(/\/+$/, "")}/updates/${file}`;
}

// A redirect nobody caches, so a new feed is read as soon as it is published.
export function redirect(location: string | null): Response {
  if (!location) return new Response(null, { status: 404 });
  return new Response(null, { status: 302, headers: { location, "cache-control": "no-store" } });
}
