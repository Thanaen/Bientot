# Contract: Built-in Activity Source

**Consumers**: `WorkTime.App` through `IActivitySourcePort`  
**Implementation**: the `GetLastInputInfo` adapter in `WorkTime.Windows`, the
idle state rules in `WorkTime.Domain`, sampling in `WorkTime.App/Services`,
span storage in `WorkTime.Persistence`  
**Status**: Default activity source since 2026-10-11 (D-021)

## General rules

- The source runs only while WorkTime runs and the built-in source is selected.
- It reads one signal: the time of the last keyboard or mouse input in the
  current Windows session. It never reads input content, key codes,
  application names, or window titles.
- Its rules match `aw-watcher-afk` on Windows (5-second polling, 180-second
  default threshold), so switching between the two sources does not change how
  a given period is classified.
- Instants come from the injected `TimeProvider`. Idle durations come from the
  system tick count, so a wall-clock change does not change them.

## Sampling

Every 5 seconds the source calls `GetLastInputInfo` and `GetTickCount`. The
idle duration is `GetTickCount() - LASTINPUTINFO.dwTime`, computed in unsigned
32-bit arithmetic so that the tick count wrapping after about 49.7 days of
uptime gives the right result.

With `T` the idle threshold (default 180 seconds, configurable from 30 seconds
to 30 minutes, FR-046) and `last input = now - idle duration`:

- idle duration `>= T` while `active`: the state becomes `idle` from the last
  input, not from the sample that detected it;
- idle duration `< T` while `idle`: the state becomes `active` from the last
  input, at most one polling interval before the sample;
- at source start, and at the first successful sample after an interruption
  (see below): `active` if idle duration `< T`, `idle` otherwise, both from
  that sample. Nothing is backdated before it.

Because idle is detected up to `T` plus one polling interval after the last
input, the time after the last input of an `active` span is pending: the next
sample either sees new input, which makes that time active, or crosses `T`,
which makes it idle from the last input. An `active` span is therefore settled
only through the last input that a sample read. An `idle` span is settled
through the latest sample that read no new input. Pending time is never
credited before it settles, and only a later sample can settle it. A clean
shutdown, a crash, a suspend, or elapsed wall time never settles it.

A threshold change applies from the change instant. Stored spans are never
rewritten, and reads never apply the current threshold to stored spans: the
settled boundary is stored with each span and does not depend on the
threshold.

Observation is uninterrupted while consecutive successful samples are no more
than two polling intervals apart. Source start, resume, a failed input query,
or a longer gap between samples (for example a stalled process) interrupts it.

## Persistence

Each `ActivitySpan` row (see [data model](../data-model.md)) holds a state,
`StartUtc`, and `SettledThroughUtc`, the latest instant at which its state is
settled as defined in [Sampling](#sampling).

- The source appends a span at each transition seen within uninterrupted
  observation. Only such a transition closes the previous span: it sets the
  previous span's `SettledThroughUtc` to the new span's `StartUtc`.
- The source also appends a span at start, at resume, and at the first
  successful sample after an interruption, even when the state is unchanged.
  Such a span starts at that sample, with nothing backdated before it, and
  leaves the previous span unchanged. The previous span keeps its stored
  `SettledThroughUtc`, so the time up to the new span is unknown.
- The stored `SettledThroughUtc` of the open span is written whenever the
  in-memory settled boundary is more than one minute ahead of it, and on a
  clean shutdown, a suspend notification, or a switch to ActivityWatch. A
  settlement that jumps by more than a minute at once, such as new input that
  settles a long sub-threshold pause, is written before reads expose it. The
  stored value advances only to the settled boundary, never to the instant of
  the shutdown, suspend, or switch. For an `active` span, that is the last
  input a sample read.
- Time between a span's `SettledThroughUtc` and the next span's `StartUtc` is
  unknown and is never credited. After a crash, at most one minute of settled
  coverage is lost and becomes unknown.
- Within uninterrupted observation, a sample in the same state appends nothing.
  Replaying a transition already stored is a no-op.

Example: the last input of an `active` span is at 10:00 and WorkTime stops
cleanly at 10:01, before `T` has elapsed. The span is stored settled through
10:00. WorkTime starts again at 11:00 and appends a new span from 11:00. The
time from 10:00 to 11:00 stays unknown on every later read, whatever the
threshold is then.

Suspend stores the open span's settled boundary at the suspend notification,
and resume starts a new span. The power history excludes the sleeping interval
in any case (FR-029). Sampling continues while the session is locked, but the
accounting matrix applies the lock rules to that time, so the activity state
does not change the result.

## Reads

`ReadAsync(start, end)` returns each stored span from `StartUtc` to
`SettledThroughUtc`, clamped to the requested range. The open span extends to
its in-memory settled boundary, not to the latest sample. The source leaves
all other time uncovered, including the live pending tail and an interrupted
tail, and never returns it as `active`. `ActivityEvidenceNormalizer` turns
uncovered time into unknown coverage and only classifies it as pending or
unresolved; it does not hold back intervals that the source returns as
`active`. The live tail is classified as pending only while it is within the
built-in horizon (`T` plus one polling interval plus jitter). An interrupted
tail is unresolved.

- The source never returns `SelectionRequired`; there is no bucket to choose.
- Health is healthy while the latest sample is no older than two polling
  intervals.
- A failed `GetLastInputInfo` call makes the source unavailable from the last
  successful sample. The summary shows a distinct degraded state and never
  treats the missing coverage as idle or active. The next successful sample
  recovers without a restart.
- A request for an instant at which ActivityWatch was the selected source is
  not sent to this source (FR-045).

## Tests

Tests use an injected clock and a fake tick reader. They cover:

- threshold crossing, exact boundary, and backdated idle start;
- return to `active` and the pending trailing edge;
- start while already idle, and no backdating before start;
- suspend in the middle of an active period, then resume;
- lock and unlock during active and idle periods;
- clean shutdown, crash, and restart with the one-minute settled bound;
- continuous sampling, a sub-threshold pause longer than one minute, new
  input that settles it, then a crash before any other write: the settled
  pause survives the reload;
- a startup span appended after an `active` span settled earlier, then a
  reload: the gap stays unknown and the earlier span is not extended;
- a missed sample interval, then recovery in the same state: a new span is
  appended and the gap stays unknown;
- clean stop with a pending tail (last input 10:00, stop 10:01, `T` not
  elapsed), read again hours later: credited activity ends at 10:00;
- crash with a pending tail, then restart: credited activity ends at the last
  stored settled boundary, and the time up to the new span stays unknown;
- a pending tail stored under one threshold, read after a threshold change: the
  result is unchanged;
- tick count wraparound;
- a threshold change in the middle of a workday;
- a failed input query and recovery.

A Windows smoke check confirms that real input moves the source between
`active` and `idle` on a packaged build.
