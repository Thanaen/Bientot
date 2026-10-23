// The homepage and its fonts, served from site/. Each file is named with a literal URL so that
// Vercel's file tracing ships it with the function.
const PAGE = new URL("../site/index.html", import.meta.url);

const FONTS: Record<string, URL> = {
  "IBMPlexSans-Regular.woff2": new URL("../site/fonts/IBMPlexSans-Regular.woff2", import.meta.url),
  "IBMPlexSans-Medium.woff2": new URL("../site/fonts/IBMPlexSans-Medium.woff2", import.meta.url),
  "IBMPlexSans-SemiBold.woff2": new URL("../site/fonts/IBMPlexSans-SemiBold.woff2", import.meta.url),
  "IBMPlexMono-Regular.woff2": new URL("../site/fonts/IBMPlexMono-Regular.woff2", import.meta.url),
  "IBMPlexMono-Medium.woff2": new URL("../site/fonts/IBMPlexMono-Medium.woff2", import.meta.url),
};

export function homepage(): Response {
  return new Response(Bun.file(PAGE), {
    headers: { "content-type": "text/html; charset=utf-8", "cache-control": "public, max-age=300" },
  });
}

export function font(name: string): Response {
  const url = Object.hasOwn(FONTS, name) ? FONTS[name] : undefined;
  if (!url) return new Response(null, { status: 404 });
  return new Response(Bun.file(url), {
    headers: { "content-type": "font/woff2", "cache-control": "public, max-age=604800" },
  });
}
