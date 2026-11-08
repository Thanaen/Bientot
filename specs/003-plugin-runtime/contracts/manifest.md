# Contract: `plugin.toml` (api 1.0, this step)

```toml
id       = "dev.bientot.focus"
name     = { en = "Focus", fr = "Concentration" }
version  = "0.1.0"
api      = "1.0"
memory   = "4MiB"          # optional, default 8MiB, at most 32MiB
locales  = ["en", "fr"]    # at least one, the first is the fallback

capabilities = ["read:day", "ui:flyout"]
events       = ["day.refreshed", "language.changed"]

[[commands]]
name    = "toggle-range"
title   = { en = "Show today or this week", fr = "Afficher aujourd’hui ou la semaine" }
windows = ["flyout", "week"]
```

## Rules

- Unknown top-level keys refuse the manifest in this step, with a message
  naming the key. Keys planned for later steps (`settings`, `timers`, `net`,
  `storage`, `wasi`, `shortcut`…) are refused with « not supported yet ».
- `id`, `version`, `api` and `locales` are required.
- `api` must be satisfiable by this host (`1.x`). A later host that adds
  primitives raises its minor version; a plugin that uses them declares that
  minor version, and an older host refuses it at install.
- `events` may list `day.refreshed` and `language.changed`. `installed`,
  `upgraded`, `activate` and `command.invoked` are always delivered.
- `day.refreshed` requires `read:day`.
- A command `title` may not equal a built-in command title in either
  language.
- Localized texts must exist for the first locale; others fall back to it.

## Validation errors

Every error names the field and the rule, in the app language, for example
« `commands[0].title`: this title is already used by Bientôt ».
