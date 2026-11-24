# Specification Quality Checklist: Linux support

**Purpose**: Validate specification completeness and quality before planning
**Created**: 2026-11-24
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details in the user stories and requirements (crates, protocols and D-Bus names are confined to `plan.md`, `research.md`, `data-model.md`, `contracts/` and `tasks.md`; the parity matrix names a mechanism only where it is the reason for a difference)
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders
- [x] All mandatory sections completed

## Requirement Completeness

- [ ] No [NEEDS CLARIFICATION] markers remain — none is written inline, but fourteen open questions wait for the owner in `lots.md`; Q2, Q3, Q4 and Q9 change requirements (FR-014, FR-022, FR-026) and block Lots 0, 9, 13 and 16
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified (twenty technical assumptions, each with the task that verifies it, in `research.md`)

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [ ] The accounting decision is recorded (D-026) before implementation (constitution, workflow rule 2) — proposed in `plan.md`, waits for Q4
- [ ] The constitution allows a second platform — amendment 1.3.0 proposed in `plan.md`, waits for Q9
- [ ] Design and wording approved by the Designer, UX Writer and Screenshot reviewers — not started; every UI and wording task is marked **[UI]** in `tasks.md` and the proposed texts are in `contracts/copy.md`

## Platform Parity

- [x] Every feature of the Windows app has a row in the parity matrix
- [x] Every « different » or « unavailable » cell says why and what the user sees
- [x] Each supported desktop has a tier and a reason
- [x] The Windows behaviour is protected by a requirement (FR-001) and a measurable criterion (SC-002)
- [ ] Every « same » cell is backed by a test or a validation step that has run — the steps exist (`quickstart.md`, harness tasks); none has run, since no Linux backend exists yet

## Notes

- Nothing in this specification was validated on a real Linux desktop. What was run is listed in `research.md`, R1: a build, its size, its libraries and a headless start in a container without a graphics driver.
- English and French texts of `contracts/copy.md` are proposals.
- Velopack file names, journal message ids and several D-Bus details are fixed by the first task of their lot, which amends the contract in the same pull request.
