# Windows session history recovery

Bientôt can read when the user's session was opened, locked, unlocked and closed
before it started watching. The read is local and read only. It uses event XML
from the official Windows Event Log API and never depends on localized message
text.

The source is the `Microsoft-Windows-Winlogon/Operational` log. Windows keeps it
by default and a standard user can read it. Winlogon logs event 811 each time
one of its subscribers starts handling a session notification. The `Event` field
of the event data names the notification:

| `Event` | Boundary |
| --- | --- |
| 2, 12 | Logon |
| 3, 13 | Logoff |
| 4, 8 | Lock (8: disconnected by Switch User) |
| 5, 9 | Unlock (9: reconnected) |

Other notifications, such as the screen saver (6, 7) and the system's own (0,
1), are not boundaries. Event 812, which marks the end of the same handling, is
not read.

The query keeps the events of the current user, by security identifier, from
24 hours before the window. Every subscriber logs each notification, so one lock
appears several times within a few milliseconds. Of a run of boundaries of one
kind, only the first is kept. The last boundary before the window tells the
state of the session when the window began.

The reader keeps at most 4,096 records. Reaching the cap, an unreadable log or a
malformed record returns no history at all, since a missing record could hide a
lock between two of the others.

`bientot-windows` implements the reader (`session_history.rs`). Its tests replay
event XML shaped as a real log renders it; the native query runs only on Windows
(`cargo test -p bientot-windows`).
