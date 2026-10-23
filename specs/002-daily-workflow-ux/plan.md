# Implementation plan

## Technical context

Use the existing C# 14, .NET 10 and WinUI 3 projects, without new dependencies.
Persist existing TimeSpan settings and append-only ManualAdjustment records.
The base is main d70df1b. The full baseline has 880 passing tests, three optional
native tests skipped and two live ActivityWatch cases excluded. Tests requiring
temporary diagnostic file access run outside the restricted sandbox.

## Constitution check

All processing stays local. ActivityWatch remains read-only. Automatic unknown
and suspended time stays excluded unless an explicit user correction says
otherwise. Native textual feedback supplements colour. Calculation and input
boundaries receive deterministic tests. No cloud feature or schema is added.

## Design

AdjustmentService owns suspension precedence. Retain Suspended only for a
ReclassificationAdjustment carrying ConfirmsReviewId; ordinary explicit interval
adjustments apply through the canonical segment split and ManualOrigin.

SettingsNumberParser owns H:mm parsing and formatting. SettingsWindow uses
TextBoxes for the two targets and submits them through the existing Targets group.

HistoryFormatter derives a raw correctable UtcInterval from each eligible entry.
HistoryWorkspace captures and validates that row before its existing queue opens
AdjustmentDialogRequest.Correct. The launcher passes an initial interval to the
existing view model. Unchanged endpoints keep their original UtcInstant values.
MainWindow opens history on today from the source warning.

WorkdayPresenter derives qualitative pending guidance from the existing indicator
and forecast confidence. Preserve projector visibility, accounting and warning
thresholds. Suppress a claim while tracking is paused or finished. New text uses
the current bilingual resource mechanism.

## Sequence and ownership

Implement suspension first, targets second, problem correction third, lunch
guidance fourth. Verify each before integration. Independent target and lunch
writers may prepare changes in separate worktrees. The root owns shared resource
integration, demo fixtures, native verification, one PR and merge readiness.

## Verification

Run focused behavior tests for each lot, then serialize full Release build and
tests. Exercise the real native app using isolated demo data. Inspect French and
English, dark and light, short windows, keyboard, invalid input, correction,
refresh, restart and confidence states. An independent reviewer verifies the PR
patch. Follow CI and review to mergeability, squash merge and fetch main.
