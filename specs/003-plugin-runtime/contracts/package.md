# Contract: `.bientot-plugin` package

## Layout

A zip file. Only these entry names are accepted; any other entry refuses the
package.

| Entry | Required | Rule |
| --- | --- | --- |
| `plugin.toml` | yes | manifest, at most 64 KiB |
| `plugin.wasm` | for a WebAssembly plugin | at most 4 MiB |
| `locales/<tag>.json` | one per declared locale | `<tag>` is a BCP 47 tag in lower case (`fr`, `en`, `en-gb`); flat object of string keys to strings, at most 64 KiB |
| `assets/<name>.svg` | no | `<name>` matches `[a-z0-9-]{1,32}`; at most 64 KiB; rendered by `resvg` with external `href` resolution off |
| `SIGNATURE` | no | see below |

## Reader rules

The reader never calls a generic extraction routine. It:

1. Reads the end of central directory record and the central directory.
2. For each entry, reads its local header and checks that name, sizes,
   method and CRC agree with the central directory.
3. Refuses: duplicate names; names equal after case folding; `..`, absolute
   paths, drive letters, UNC paths, `\`, `:`, reserved Windows names (`CON`,
   `NUL`, `COM1`…), trailing dots or spaces; symbolic links and directories
   with content outside the allowed names; encryption; methods other than
   *stored* (0) and *deflate* (8); ZIP64.
4. Caps: package at most 8 MiB, at most 256 entries, each entry's
   uncompressed size at most its limit above, compression ratio at most 100.
5. Inflates accepted entries into memory with `flate2`, checks CRC-32, then
   writes them to a temporary folder under `plugins\` and renames it to
   `plugins\<id>\` in one step.

## Signature

`SIGNATURE` is UTF-8 text:

```text
bientot-plugin-signature 1
key ed25519 <public key, base64>
sig <signature, base64>
```

The signed message is the canonical file list, UTF-8, one line per entry
other than `SIGNATURE`, sorted by path in byte order:

```text
<sha256 hex> <path>\n
```

A package without `SIGNATURE` is unsigned. A package whose signature does not
verify, or whose file list does not match its entries exactly, is refused.

The key fingerprint shown to users is the first 16 bytes of the SHA-256 of
the public key, in groups of four hex digits.

## Trust on install and update

- First install of an id: bind the id to the key (signed) or to the package
  hash (unsigned).
- Later package for that id, signed: accepted if signed by the bound key, or
  by a key the bound key announced. An announcement is a line
  `announce ed25519 <new key> <signature by the old key over the new key>` in
  `SIGNATURE`.
- Later package for an unsigned id: treated as a new install; the old one is
  removed with its state after the user confirms.

## Load-time check

Each time a plugin loads, the host hashes its files and compares them with
`InstalledPlugin.files`. Any difference sets `FilesChanged` and the plugin
does not run.
