# Contract: usage signal on two platforms

Today (`services/bientot/src/ping.ts`, `docs/privacy.md`): at most one
request a day, with exactly three keys. The service refuses any other key
set and any body over 256 bytes.

```json
{ "version": "1.0.214", "channel": "direct", "first_this_month": true }
```

`channel` is one of `store`, `direct`, `zip`.

## Minimum change (needed whatever Q3 decides)

`channel` gains two values. They tell the platform by themselves.

| Channel | Build |
| --- | --- |
| `store` | Microsoft Store (MSIX) |
| `direct` | Windows installer (Velopack) |
| `zip` | Windows portable ZIP |
| `flatpak` | Flathub |
| `appimage` | AppImage (Velopack) |

A development build sends nothing, as today. Nothing else changes: same
three keys, same cadence, same opt-out, same server-side storage.

The service is deployed **before** any Linux build that sends, because an
unknown channel is rejected (and a rejection stops further sends for that
launch, by design).

## Recommended addition (open question Q3)

Two optional keys, sent by Linux builds only:

```json
{
  "version": "1.0.214",
  "channel": "flatpak",
  "first_this_month": false,
  "desktop": "gnome",
  "session": "wayland"
}
```

| Key | Values (closed list) | Derived from |
| --- | --- | --- |
| `desktop` | `gnome`, `kde`, `cinnamon`, `xfce`, `other` | `XDG_CURRENT_DESKTOP`, mapped on the device; any value not in the list is sent as `other`, never as text |
| `session` | `wayland`, `x11` | Session type |

Why: Linux is several desktops with different backends (`linux-backends.md`).
These two fields say where users are, so that testing effort and the Tier
list follow reality rather than guesses. Why it is a question: it is new
data leaving the machine (Principle I). It identifies nobody: five values
by two, under the existing small-number suppression of the statistics
page.

Not proposed, deliberately: distribution name or version, kernel,
architecture (the download counts of each AppImage give it), compositor
version, GPU, locale.

If accepted, in this order:

1. `docs/privacy.md` and the site's privacy page list the two keys and
   their values, French and English (T160).
2. The service accepts the key sets `{3 keys}` and `{3 keys + desktop +
   session}`, the latter only with a Linux channel; `MAX_BODY_BYTES` stays
   256 (T161).
3. The app sends them (T162).
4. The statistics page and the PostHog saved query (`usage_daily`) gain
   the two dimensions with the same suppression threshold; the query's
   version becomes 2 (T163).

If refused: steps 1 to 4 reduce to the channel values, and the Tier list is
revised from issue reports alone.

## Tests (T164)

- Service: each new channel accepted; `desktop` or `session` with a
  Windows channel refused; a free-text `desktop` refused; one of the two
  keys without the other refused.
- App: the mapping of `XDG_CURRENT_DESKTOP` (`ubuntu:GNOME` → `gnome`,
  `KDE` → `kde`, `X-Cinnamon` → `cinnamon`, `XFCE` → `xfce`, `sway` →
  `other`, unset → `other`); a Windows build's body is byte-identical to
  today's.
