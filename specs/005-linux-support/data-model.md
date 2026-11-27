# Data Model: Linux support

No accounting type changes. This feature adds the data the ports exchange,
a few stored values, and names for Linux evidence in the ledger.

## Types exchanged through the ports

Signatures are in `contracts/platform-ports.md`. Meaning and rules:

| Type | Fields | Rules |
| --- | --- | --- |
| `LastInputReading` (existing, domain) | `tick_count: u32`, `last_input_tick: u32` | Unchanged. On Linux built by `reading_from_idle(uptime, idle)`: `tick_count = uptime_ms as u32`, `last_input_tick = tick_count.wrapping_sub(idle_ms as u32)`. `idle` is clamped to `uptime` |
| `LifecycleChange` (existing) | `Session(SessionState)` or `Power(PowerState)` | Linux emits the same four changes as Windows and no other |
| `SessionSnapshot` | `session`, `lock_reporting` | `lock_reporting = Unavailable` implies `session = Unknown` |
| `PowerBoundary` | `kind`, `at`, `source_key` | `source_key` is unique per boundary and stable across reads; recording is idempotent on it, as on the Event Log record id today |
| `SessionBoundary` | `kind`, `at`, `source_key` | Linux yields at most one, of kind `Logon` |
| `LockHistory` | `Recorded`, `NotRecorded` | Property of the log, not of a read. `NotRecorded` switches the recovery to the rule of D-026 |
| `TrayHost` | `Present`, `Absent` | `Absent` only after 5 s without a host, so a restarting shell does not flip the mode. On Linux the host is the StatusNotifierWatcher, the only part a client sees; at start, what the watcher answers, without the 5 s |
| `FlyoutPlacement` | `At`, `Layer`, `CompositorPlaced`, `Window` | Chosen once per opening. `Window` if and only if `TrayHost::Absent` |
| `InstallKind` | `Store`, `Direct`, `Portable`, `Flatpak`, `AppImage`, `Development` | Decides: update source (`Direct`, `AppImage`: Velopack; `Store`: Store; others: none), startup backend, usage-signal channel (`Development` sends nothing) |
| `SystemDescription` | `distribution`, `distribution_version`, `desktops`, `session_type` | Diagnostics only (plus the two optional usage-signal keys if Q3 is accepted); the architecture stays in the snapshot's `runtime` |

### State of each capability

Shown to the user when not available (`spec.md`, FR-009, FR-011, FR-012).

```text
Tray          Present ⇄ Absent                (live, from the host)
Idle          Backend(name) ⇄ None            (re-selected when the backend stops answering, and each minute while none)
Lock          Live ⇄ Unavailable              (per session; may become Live on the first true LockedHint)
PowerHistory  Journal | BootTimeOnly          (per start)
Notifications Available ⇄ Unavailable         (re-checked when a show fails)
Startup       StartupRegistrationState        (read when the settings open and after a change)
```

## Stored values

### `settings.json`

No new key. `schemaVersion` stays 1. A file written on one platform is
valid on the other; keys that have no meaning there are kept untouched
(the update settings in a Flatpak, for example).

### `app-state.json`

Two optional keys, written on Linux only, ignored and preserved by
Windows:

| Key | Type | Meaning |
| --- | --- | --- |
| `windowModeNoticeDismissed` | bool | The user closed the notice of FR-012; it does not return while the desktop stays the same. Written only once true, so that Windows writes the file it always wrote; anything but `true` reads as false. Which desktop it was is not kept yet: the notice (T064) decides whether it needs to be |
| `autostartGranted` | bool or absent | Flatpak only: the Background portal's last answer |

### Ledger

No migration. Linux evidence uses the existing tables with these origin
strings, beside `windows-power-history` and the Windows session-history
origin:

| Origin | Written by | Rows |
| --- | --- | --- |
| `linux-power-history` | Journal reader | Suspend, resume, shutdown and boot boundaries |
| `linux-boot-time` | Fallback without a journal | The current boot boundary |
| `linux-session-history` | First-start recovery | The session opening, and the « Avant Bientôt » review |

Live lifecycle transitions and activity spans carry no platform mark: they
are the same facts. Readers of the ledger treat an unknown origin as they
do today (kept, shown as evidence); a test proves that a ledger with Linux
origins is read without loss by a Windows build and the reverse.

Operation keys keep using `dotnet_ticks`, unchanged.

### Time-zone identifiers

The ledger stores the zone id that was local when a row was written:
Windows key names on Windows (« Romance Standard Time »), IANA names on
Linux (« Europe/Paris »). Both stay as written. Resolution on read accepts
either on both platforms through the CLDR `windowsZones` table (task T125,
assumption A-14).

### Files outside the data folder (Linux)

| Path | Owner | Removed by |
| --- | --- | --- |
| `$XDG_CONFIG_HOME/autostart/dev.bientot.Bientot.desktop` | Startup switch (AppImage) | Turning the switch off |
| `~/.local/share/applications/dev.bientot.Bientot.desktop`, icons under `~/.local/share/icons/hicolor/` | AppImage first run | « Remove from the app menu » |
| `$XDG_RUNTIME_DIR/bientot/<stem>.sock`, `.lock` (stem: the instance key's first 41 characters); `<data folder>/run/` without `$XDG_RUNTIME_DIR` | Single instance | The socket at exit, or by the next owner after a crash; the lock stays; the runtime directory is cleared at sign-out by the system |

`docs/privacy.md` lists them (T183).
