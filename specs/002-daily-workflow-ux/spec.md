# Daily workflow improvements

**Feature branch**: `codex/daily-workflow-ux`
**Created**: 2026-09-15
**Status**: Approved for autonomous implementation
**Input**: Four coherent daily workflow lots requested by the user, grounded in
issues 65,
70,
45 and
53.

## User scenarios and testing

### US1. Declare work during a suspended period, P1

As a user who works away from a suspended PC, I can explicitly declare that work.
The automatic suspension rule must still exclude all uncorrected time.

Independent test. Correct a suspended hour, restart, and inspect daily and weekly totals.

1. Given suspended coverage, adding a worked interval or reclassifying it as
   worked credits exactly the selected coverage once and retains the manual label.
2. Given a partially overlapping suspension, only the selected portion changes.
3. Given a saved correction with a reason, editing and reopening preserves its
   provenance, reason, duration and deterministic effect on weekly credit.
4. Confirming a long locked episode does not implicitly credit a suspension.

### US2. Enter targets in hours and minutes, P1

As a user, I can enter a 7-hour 24-minute target as `7:24`.

Independent test. Save `7:24`, observe the recalculated target, then restart.

1. Daily and minimum targets display and accept hours and minutes in both languages.
2. Values range from `0:01` to `24:00`. Minutes must have two digits between 00
   and 59. Blank, decimal, malformed and out-of-range inputs remain visible with
   a useful error and do not replace the last saved settings.
3. The minimum cannot exceed the daily target. Both values save atomically.

### US3. Inspect and correct exact problem periods, P1

As a user seeing missing activity, I can find each affected period and correct
it without guessing dates or times.

Independent test. Open a warning with two disjoint unknown periods, choose one,
save its correction and confirm that the other period remains unresolved.

1. The source warning provides an action to today's history. Separate gaps remain
   separate and never imply that worked time between them is missing.
2. Unknown and suspended automatic periods have a direct correction action.
   The dialog identifies the selected period and preselects reclassification.
   Saving requires an explicit user action. Cancellation changes nothing.
3. An unchanged prefilled period preserves its exact original instants, including
   seconds, midnight endpoints and repeated local clock times.
4. Stale or closed-week rows cannot authorize a correction. Existing manual
   corrections use the existing edit path. Long-lock confirmation remains distinct.

### US4. Understand the outlook before lunch, P2

As a user before lunch, I can see whether the planned pause leaves comfortable
margin, little margin, or would require a departure after my preferred time.

Independent test. Compare early, near-limit and late starts before lunch.

1. Guidance includes the planned lunch exclusion and assumes continued work
   outside the planned pause. It does not reveal the hidden exact departure time.
2. The existing warning margin defines comfortable versus little margin.
   Exact preferred-time completion remains achievable with little margin.
3. Estimated evidence is identified. Unavailable evidence gives no confident
   assurance. Non-working, complete, paused and finished states retain precedence.
4. During lunch, guidance concerns the remaining planned pause. At lunch end,
   the existing exact departure display takes over.

### Edge cases

Cover partial suspension overlap, supersession, retries, overlapping manual work,
week rollover, subminute source gaps, repeated local times, `24:00`, invalid
drafts, zero warning margin, and targets already reached before lunch.

## Requirements

- FR-001. Preserve local storage, source privacy and current-week boundaries.
- FR-002. Explicit manual interval work and reclassification may override automatic
  suspension. General long-lock confirmation must preserve suspended coverage.
- FR-003. Enter target durations as hours and minutes with explicit validation.
- FR-004. Show exact correctable periods through the existing history journey.
- FR-005. Preserve exact prefilled instants until the corresponding input changes.
- FR-006. Show conditional qualitative lunch guidance without guessing unknown work.
- FR-007. Every new label, action and error must have French and English resources.
- FR-008. Keyboard users must reach the actions and receive visible error feedback.

### Key entities

- Manual correction. A dated user declaration with a reason and original provenance.
- Problem period. An exact portion of a daily breakdown needing user attention.
- Target duration. A duration expressed to the user in hours and minutes.
- Lunch outlook. A conditional assessment using existing target and departure rules.

## Success criteria

- SC-001. A selected one-hour suspension correction adds exactly one worked hour
  after both the initial save and restart, without duplicate weekly credit.
- SC-002. Entering `7:24` produces a 7-hour 24-minute target after restart.
- SC-003. A user can reach a prefilled correction from a warning in two actions.
- SC-004. Three pre-lunch scenarios produce distinct, truthful guidance in both languages.
- SC-005. Native dark and light windows keep the four journeys readable and usable
  by keyboard, including a short window that scrolls naturally.

## Assumptions and scope

This extends the ratified core feature. The user delegated product choices within
these reported needs. No onboarding, update client, dependency refresh, release
signing or existing independent PR is included.
