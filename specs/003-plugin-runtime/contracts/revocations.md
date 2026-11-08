# Contract: revocation list

## Address and request

`GET https://bientot.dev/plugins/revocations.json`, no query string, no
cookie, no identifier, the same `User-Agent` as the update check. The
response is at most 256 KiB.

When Bientôt reads it:

- with the daily update check, when « Check for updates » is on (Store and
  Velopack builds);
- when the user installs a plugin or turns one on, on every build, the
  portable ZIP included;
- never in a debug build or a `--demo` run.

When it cannot be read (network error, bad signature, older version, expired),
Bientôt keeps the last good list, logs `plugins.revocations.unavailable` with
the reason, and, during an install or a turn-on, tells the user it used the
last list it had.

## Format

```json
{
  "version": 3,
  "expires": "2026-12-31T00:00:00Z",
  "level2_disabled": false,
  "revoked": [
    { "id": "com.example.bad", "reason": { "en": "Sends data without consent", "fr": "Envoie des données sans consentement" } },
    { "key": "ab12-cd34-ef56-7890-ab12-cd34-ef56-7890", "reason": { "en": "Signing key leaked", "fr": "Clé de signature compromise" } }
  ],
  "signature": "<ed25519, base64, over the canonical JSON of every other field>"
}
```

Canonical JSON: keys sorted, no insignificant whitespace, UTF-8, numbers as
integers.

## Rules

- The verifying public key is built into the executable; the private key is
  kept offline by the maintainer (`docs/code-signing.md`).
- A list whose `version` is lower than the last good one, or whose `expires`
  is past, is ignored.
- A revoked `id` or `key` turns matching plugins to `Revoked` with the
  reason; a later list that drops them turns them to `Off`, not back on.
- `level2_disabled: true` stops every WebAssembly plugin until a later list
  says `false`.
- The first list is empty, with an expiry one year ahead; each publication
  increments `version`.
