# Contract: `plugin.json` (api 1.0, this step)

The manifest is JSON: `toml` would add about 195 KiB to the executable,
`serde_json` is already in it (`research.md`, R10).

```json
{
  "$schema": "https://bientot.dev/plugins/manifest-1.json",
  "id": "bientot.focus",
  "name": { "en": "Focus", "fr": "Concentration" },
  "description": { "en": "Your longest focus today", "fr": "Votre plus longue concentration du jour" },
  "version": "0.1.0",
  "api": "1.0",
  "memory": "4MiB",
  "locales": ["en", "fr"],
  "capabilities": ["read:day", "ui:flyout"],
  "events": ["day.refreshed", "language.changed"],
  "commands": [
    {
      "name": "toggle-range",
      "title": { "en": "Show today or this week", "fr": "Afficher aujourd’hui ou la semaine" },
      "windows": ["flyout", "week"]
    }
  ]
}
```

## Keys

| Key | Required | Rule |
| --- | --- | --- |
| `$schema` | no | a string, ignored; lets an editor check the file |
| `id` | yes | plugin id (data model); ids starting with `dev.` are reserved |
| `name` | yes | localized, at most 40 characters |
| `description` | no | localized, at most 120 characters |
| `version` | yes | `major.minor.patch` |
| `api` | yes | `1.x` with x at most the host's minor (`1.0` here) |
| `memory` | no | `"<n>KiB"` or `"<n>MiB"`, default `8MiB`, at most `32MiB` |
| `memory_min` | no | same form, default `1MiB` (or `memory` if smaller), at most `memory` |
| `locales` | yes | lower-case BCP 47 tags, at least one; the first is the fallback |
| `capabilities` | no | `read:day`, `ui:flyout` |
| `events` | no | `day.refreshed`, `language.changed` |
| `commands` | no | at most 8 objects `{ name, title, windows }` |
| `wasi` | no | `false` only |

A localized text is an object of locale tags to strings, and must have a
non-empty text for the first locale.

A command's `name` matches `^[a-z][a-z0-9-]{0,31}$` and is unique; its
`title` is localized, at most 48 characters; `windows` lists `flyout`,
`week` or both, both by default.

## Rules

- Unknown keys refuse the manifest, with a message naming the key. Keys
  planned for later steps (`settings`, `storage`, `timers`, `net`, `slots`…)
  are refused with « not supported yet ».
- `api` must be satisfiable by this host. A later host that adds primitives
  raises its minor version; a plugin that uses them declares that minor
  version, and an older host refuses it at install.
- `installed`, `upgraded`, `activate` and `command.invoked` are always
  delivered and are not listed in `events`.
- `day.refreshed` requires `read:day`.
- A command `title` may not equal a built-in command title in either
  language, ignoring case and surrounding spaces.
- A `shortcut` key in a command is refused: plugins do not set keyboard
  shortcuts.
- The file is at most 64 KiB.

## Validation errors

Every error names the field and the rule, in the app language, for example
« `commands[0].title`: this title is already used by Bientôt ».
