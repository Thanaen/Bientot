// Publishes the output of scripts/release-direct.sh to the Blob store the service points at:
//
//   BLOB_READ_WRITE_TOKEN=… bun scripts/publish-release.ts <output-dir>
//
// The packages go first and the feed last, so an installed WorkTime never reads a feed whose
// package is missing. Packages keep their name for good: one already in the store, such as the
// previous release vpk downloads to compute the delta, is left as it is, and one of the same name
// but another size stops the publication. The feed, the installer and the portable ZIP are
// replaced, and cached for a minute at most. The URLs it prints start with the store's public
// URL, the service's RELEASES_URL.
import { readdir } from "node:fs/promises";
import { join } from "node:path";
import { BlobNotFoundError, head, put } from "@vercel/blob";

const directory = process.argv[2];
if (!directory) throw new Error("usage: publish-release.ts <output-dir>");

const FEED = "releases.win.json";
const files = (await readdir(directory)).filter((name) => /\.(nupkg|exe|zip|json)$|^RELEASES$/.test(name));
if (!files.includes(FEED)) throw new Error(`${directory} has no ${FEED}`);
const order = [...files.filter((name) => name.endsWith(".nupkg")), ...files.filter((name) => !name.endsWith(".nupkg") && name !== FEED), FEED];

/** The size of the package already published under `pathname`, or `null`. */
async function publishedSize(pathname: string): Promise<number | null> {
  try {
    return (await head(pathname)).size;
  } catch (error) {
    if (error instanceof BlobNotFoundError) return null;
    throw error;
  }
}

for (const name of order) {
  const pathname = `updates/${name}`;
  const file = Bun.file(join(directory, name));
  const replaced = !name.endsWith(".nupkg");
  if (!replaced) {
    const size = await publishedSize(pathname);
    if (size === file.size) {
      console.log(`${name} already published`);
      continue;
    }
    if (size !== null) throw new Error(`${name} is already published with other contents (${size} bytes)`);
  }
  const blob = await put(pathname, file, {
    access: "public",
    addRandomSuffix: false,
    allowOverwrite: replaced,
    multipart: true,
    ...(replaced ? { cacheControlMaxAge: 60 } : {}),
  });
  console.log(blob.url);
}
