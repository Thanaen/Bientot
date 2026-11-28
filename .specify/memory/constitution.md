# WorkTime Constitution

> Written when the app was called WorkTime; it is now Bientôt.

## Core Principles

### I. Local-first privacy

WorkTime MUST perform its calculations and persist its configuration locally.
It MUST read only the activity data required for work-time calculation, from
the built-in source or, when selected, from ActivityWatch.
Window titles, application names, and raw activity history MUST NOT be copied
into WorkTime storage or transmitted elsewhere. Any future networked feature
requires an explicit product decision and opt-in.

### II. Explainable time accounting

Every displayed status, projection, and overtime value MUST be reproducible from
a documented rule set and a visible daily breakdown. Credited, excluded, and
labelled intervals MUST remain distinguishable. Ambiguous or missing source data
MUST be surfaced; it MUST NOT be silently guessed as worked time.

### III. Tray-first calm UX

The primary experience MUST remain available from the system tray of the
desktop (the notification area on Windows, a StatusNotifierItem host on Linux)
and must not require a permanently open window. Where a Linux desktop has no
tray host, Bientôt says so and runs in a window; that fallback is never the
default on a desktop that has one. The application MUST communicate the
current state at a glance, remain usable with colour-vision deficiencies, and
avoid unsolicited notifications by default. Settings and diagnostic detail MAY
use a conventional window.

### IV. Resilient activity source boundary

Activity sources MUST sit behind one port. The built-in source is the default.
ActivityWatch is optional, and its HTTP API and bucket naming are an external
boundary. That boundary MUST be isolated behind an adapter, covered by contract
fixtures, and able to report disconnected, incompatible, and stale states.
WorkTime MUST never modify ActivityWatch-owned buckets.

### V. Simple, deterministic core

Business rules MUST live in a UI-independent deterministic calculation core.
Time, timezone, and source events MUST be injectable in tests. Prefer the
libraries of the platform and of the Rust ecosystem already in the dependency
tree over extra dependencies; every dependency requires a concrete maintenance
or reliability benefit.

### VI. Contained extensions

Third-party code MUST never run on the UI thread, inside the accounting
refresh, or with ambient authority. Extensions run in a sandbox with bounded
fuel, memory and time, and reach the app only through host functions tied to
capabilities the user granted. The host draws every extension surface with its
own components and design-system tokens. An extension MAY bring evidence or
proposals; only the calculation core counts time, and everything an extension
caused stays attributed, visible and reversible. No extension may abort the
process. With no extension installed, the app MUST pay no runtime cost. An
extension capability that sends data off the machine requires its own product
decision (Principle I).

### VII. Platform parity

Windows and Linux are equal. A feature is designed, specified, built and
tested for both before it ships on either, unless the platform cannot provide
it; that case is written in the parity matrix of `docs/platforms.md` with what
the user sees instead.

- Every pull request compiles, lints and tests both platforms. Neither set of
  checks is optional for a change to shared code.
- Platform-specific code lives in the platform crates and in the one
  composition root of the app. A platform condition anywhere else is a defect;
  CI refuses it.
- A specification states the behaviour on each platform. « Same » is a claim
  that a test or a validation step backs.
- Accounting rules are identical. Where the evidence a platform can give
  differs, the difference is a recorded product decision and is visible to the
  user.
- Wording never names the other platform.

## Product and quality constraints

- Bientôt supports Windows and Linux. The supported versions, distributions
  and desktops are listed in `docs/platforms.md` and change by pull request.
- The default installation and runtime require no cloud account.
- Core calculations require boundary-focused unit tests, including lunch-window
  overlap, midnight, daylight-saving transitions, and exact colour thresholds.
- ActivityWatch integration requires recorded contract fixtures and an
  end-to-end smoke test against a supported local ActivityWatch version.
- The built-in activity source requires injected-clock tests and a Windows
  smoke check with real input.
- Session-lock tracking requires a Windows integration test or a documented
  manual release checklist where automation is impractical.
- Accessibility is a release criterion: state is conveyed by text/icon shape or
  tooltip in addition to colour.

## Development workflow

1. Product behaviour is specified before implementation.
2. Open decisions that affect accounting semantics block implementation of the
   relevant slice.
3. Each feature is delivered as an independently testable vertical slice.
4. Pull requests include tests proportional to behavioural risk and update the
   specification when user-visible rules change.
5. Release artifacts are built by CI from a tagged commit.

## Governance

This constitution overrides convenience decisions in plans and tasks. Changes
require a documented rationale and an update to affected specifications. The
initial constitution becomes ratified when the first product specification is
approved.

**Version**: 1.3.0 | **Ratified**: 2026-07-27 | **Last Amended**: 2026-11-28

Amendment 1.1.0 (2026-10-11): ActivityWatch is no longer a prerequisite. The
owner chose a built-in idle detection source as the default (decision D-021 in
`docs/product-decisions.md`), so Principles I and IV now name activity sources
in general. Affected specifications: `specs/001-core-workday-indicator/`.

Amendment 1.2.0 (2026-11-08): the owner chose to open Bientôt to extensions
(decision D-024 in `docs/product-decisions.md`). Principle VI states the
rules every extension plan is checked against. Affected specifications:
`specs/003-plugin-runtime/`.

Amendment 1.3.0 (2026-11-28): the owner made Linux a platform equal to
Windows. Principle VII states parity; Principles III and V and the platform
constraint no longer name Windows alone. Decision D-026 in
`docs/product-decisions.md` records the evidence Linux gives. Affected
specifications: `specs/005-linux-support/`.
