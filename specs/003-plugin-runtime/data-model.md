# Data Model: Plugin runtime

All types live in `bientot-plugin-api` unless noted. Every enum that crosses
the host–plugin boundary has an `Unknown` variant, and no struct uses
`deny_unknown_fields`, so that additions inside a major `api` version do not
break older plugins.

## PluginId

A string matching `^[a-z0-9]+(\.[a-z0-9-]+){1,4}$`, at most 64 characters,
for example `dev.bientot.focus`. The prefix `dev.` is reserved for the later
developer mode. It becomes a folder name, a settings key and a log file
name, so nothing else is accepted.

## Manifest

Parsed from `plugin.toml` (contract `manifest.md`).

| Field | Type | Rule |
| --- | --- | --- |
| `id` | PluginId | |
| `name` | localized text | at most 40 characters per locale |
| `version` | semver | |
| `api` | semver requirement | this host supports `1.x` |
| `memory` | size | default 8 MiB, at most 32 MiB |
| `memory_min` | size | default 1 MiB, at most `memory` |
| `locales` | list of BCP 47 tags | at least one; the first is the fallback |
| `capabilities` | set of Capability | this step: `read:day`, `ui:flyout` |
| `events` | set of EventKind | `installed`, `upgraded`, `activate`, `command.invoked` are implied |
| `commands` | list of CommandDecl | at most 8 |
| `wasi` | bool | must be `false` in this step |

**CommandDecl**: `name` (`^[a-z][a-z0-9-]{0,31}$`), `title` (localized, at
most 48 characters, never a built-in title), `windows` (`flyout`, `week` or
both). A `shortcut` key is refused.

## Capability

`ReadDay`, `UiFlyout`, `Unknown(String)`. Each maps to one host interface:

| Capability | Interface | Consent text (EN / FR) |
| --- | --- | --- |
| `read:day` | `bt:day@1` | « Read your current day » / « Lire votre journée en cours » |
| `ui:flyout` | `bt:ui@1` (slot `flyout.card`) | « Show a card in the flyout » / « Afficher une carte dans le flyout » |

`bt:log@1` and `bt:time@1` need no capability.

## InstalledPlugin

Stored in `plugins/state.json` (host-private, not part of the API crate).

| Field | Meaning |
| --- | --- |
| `id`, `version` | from the manifest |
| `trust` | `Signed { key_fingerprint, announced_keys }` or `Unsigned { package_hash }` |
| `files` | path → SHA-256, from the signed list or computed at install |
| `manifest_hash` | SHA-256 of `plugin.toml` |
| `granted` | capabilities the user accepted, with the `manifest_hash` they were accepted for |
| `enabled` | the user's switch |
| `status` | `Active`, `Off`, `FailedUntilRestart { reason }`, `Revoked { reason }`, `FilesChanged`, `UnsupportedApi` |
| `failures` | timestamps of the last traps (at most 3 kept) |
| `card_preference` | the user picked this card as the visible one |
| `hidden_on` | local date the card was hidden for |

State transitions:

```text
            install (consent)             turn off
 (none) ─────────────────────► Active ◄──────────────► Off
                                  │  ▲     turn on
          3 traps in 10 min       │  │  next start
                                  ▼  │
                       FailedUntilRestart
 Active/Off ── revocation list names it ──► Revoked ── list clears it ──► Off
 Active/Off ── file hash differs at load ──► FilesChanged ── reinstall ──► Active
 Active/Off ── host no longer supports api ──► UnsupportedApi
 any ── remove ──► (none, folder deleted)
```

## RevocationList

| Field | Meaning |
| --- | --- |
| `version` | integer, only increases |
| `expires` | instant; a list past it is ignored |
| `revoked` | list of `{ id?, key_fingerprint?, reason { en, fr } }` |
| `level2_disabled` | bool |
| `signature` | ed25519 over the canonical JSON of the other fields |

The host keeps the last good list in `plugins/revocations.json`.

## DaySnapshot (v1)

Built from `WorkdayRefreshState` and the published week, only when a plugin
listens.

| Field | Type |
| --- | --- |
| `schema` | `1` |
| `computed_at` | UTC instant |
| `time_zone` | IANA name |
| `date` | local date |
| `language` | `fr`, `en` |
| `tracking` | `Running`, `Paused`, `Finished`, `NonWorkingDay`, `Unknown` |
| `target`, `minimum`, `counted` | durations in seconds |
| `departure_at_target`, `departure_at_minimum` | UTC instant or null (reached, or no forecast) |
| `forecast` | `Firm`, `Estimated`, `Unavailable`, `Unknown` |
| `segments` | list of `{ start, end, kind }`, settled only; `kind` is `Work`, `Break`, `Lunch`, `Away`, `Manual`, `Appointment`, `Unknown`, `Other` |
| `open_tail` | `{ start, kind }` or null |
| `week` | `{ counted, target, credit }` in seconds |
| `source` | `Ok`, `Degraded`, `Unknown` |

The departure at the minimum uses the same projection as the departure at
the target (`bientot_domain::forecasting::project_departure`) with the
minimum as its target. Review ids, adjustment ids and other internals never
appear.

## View tree

See `contracts/view-tree.md`. A **prepared card** is the validated tree
converted into the app's widgets, with the plugin's icon and name, built once
per update and stored in the contribution set.

## ContributionSet (`bientot-app`)

A GPUI `Global` holding an `Arc` of: the prepared card of each active plugin
(or none), and the declared commands of each active plugin. The plugin
thread's updates replace it as a whole; views only read it.
