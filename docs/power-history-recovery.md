# Windows power history recovery

WorkTime reads the Windows System event log before it calculates the first
summary after startup. The read is local and read only. It uses event XML from
the official Windows Event Log API and never depends on localized message text.

The accepted boundaries are deliberately narrow:

- Kernel-General event 13 supplies a shutdown boundary from `StopTime`.
- Kernel-General event 12 supplies a boot boundary from `StartTime`.
- Kernel-Power event 42 supplies a suspend boundary from `TimeCreated`.
- Kernel-Power event 107 supplies a resume boundary from `TimeCreated`.

Only an unambiguous shutdown followed by a boot, or suspend followed by a
resume, creates an excluded interval. Missing, malformed, conflicting, or
out-of-order boundaries create no interval. A boot without a shutdown clears
old session knowledge but does not invent powered-off time.

The reader scans the matching log to EOF and retains at most 4,096 records. It
does not filter by `TimeCreated`, because Kernel-General `StartTime` and
`StopTime` are authoritative and can be ordered differently. Reaching the cap
before EOF returns no historical intervals as Truncated. Startup allows three
seconds for the read, then continues with conservative unknown session and power
boundaries. These
boundaries stop a lock or suspend observed before the workday from standing in
for missing history. A real observation at the exact workday boundary is applied
after the reset and remains authoritative. A real in-window lock still produces
provisional review time even when power continuity is unknown. Active and idle
periods recorded by the built-in source also remain usable after a reset. A
gap without such evidence remains unknown.

Every persisted boundary key includes its provider, event ID, record ID, and UTC
timestamp. Re-reading the same history is idempotent, including after the System
log is cleared and record IDs are reused. Both ends of one recovered interval
are written in one SQLite transaction.

`worktime-windows` implements the reader (`power_history.rs`). Its tests
replay sanitized event XML and cover the boundary rules above; the native
event log read runs only on Windows (`cargo test -p worktime-windows`).
