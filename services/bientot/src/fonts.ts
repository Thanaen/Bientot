// Written by scripts/fonts/subset.py: the font files the pages use, named by their content, so that
// browsers may keep them for good. Each one is named with a literal URL for Vercel's file tracing.
export const FONTS: Record<string, URL> = {
  "/fonts/sans-400-core.cbc478cc.woff2": new URL("../site/fonts/sans-400-core.cbc478cc.woff2", import.meta.url),
  "/fonts/sans-400-ext.a26eeb58.woff2": new URL("../site/fonts/sans-400-ext.a26eeb58.woff2", import.meta.url),
  "/fonts/sans-500-core.ff0e212e.woff2": new URL("../site/fonts/sans-500-core.ff0e212e.woff2", import.meta.url),
  "/fonts/sans-500-ext.800cc717.woff2": new URL("../site/fonts/sans-500-ext.800cc717.woff2", import.meta.url),
  "/fonts/sans-600-core.2f3f51a4.woff2": new URL("../site/fonts/sans-600-core.2f3f51a4.woff2", import.meta.url),
  "/fonts/sans-600-ext.60d6d344.woff2": new URL("../site/fonts/sans-600-ext.60d6d344.woff2", import.meta.url),
  "/fonts/mono-400-core.d5bb6b68.woff2": new URL("../site/fonts/mono-400-core.d5bb6b68.woff2", import.meta.url),
  "/fonts/mono-400-ext.5242ecfc.woff2": new URL("../site/fonts/mono-400-ext.5242ecfc.woff2", import.meta.url),
  "/fonts/mono-500-core.fc127d43.woff2": new URL("../site/fonts/mono-500-core.fc127d43.woff2", import.meta.url),
  "/fonts/mono-500-ext.1ca74e17.woff2": new URL("../site/fonts/mono-500-ext.1ca74e17.woff2", import.meta.url),
};
