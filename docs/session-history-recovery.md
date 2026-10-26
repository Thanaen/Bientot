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

## Recording and accounting

`SessionHistoryRecovery` records the boundaries from the start of the workday to
the moment the app starts watching, in one transaction. Logon and unlock record
an unlocked session, lock a locked one, and logoff an unknown one: a closed
session tells nothing about the user. Each key starts with `session-history:`
and names the record ID and the UTC instant, so reading the same history again
records nothing new. The ledger gives these transitions the origin
`windows-session-history`. A read that fails, times out after three seconds or
cannot be recorded records nothing.

The accounting credits provisionally, with the label `RebuiltWork`, the time
without any activity evidence while such an unlock holds, outside lunch. The
session state Bientôt observes when it starts replaces it, so the rule stops
where Bientôt starts watching. Each run of rebuilt time has its own review, with
the reason `RebuiltSession`, so that an answer never covers the lunch or the
sleep between two runs. The day cannot be finished at such a review. A lock read
from the log is a lock like any other: it counts as locked work or lunch, and a
long one asks for a review.

`bientot-windows` implements the reader and the recovery (`session_history.rs`).
Its tests replay event XML shaped as a real log renders it and account the
recorded history; the native query runs only on Windows
(`cargo test -p bientot-windows`).
