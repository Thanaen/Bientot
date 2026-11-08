# Specification quality

- [x] Five independently testable user journeys are defined, prioritized.
- [x] Requirements state outcomes and limits; each story has acceptance scenarios.
- [x] Success criteria are measurable against the existing release measurements.
- [x] The constitution check passes, including the new Principle VI.
- [x] Privacy: no plugin capability sends data; the only new request is ratified by D-024 and listed for `docs/privacy.md`.
- [x] Accounting: plugins only read the published day; `bientot-domain` is untouched.
- [x] Scope boundaries are explicit, and the later steps that need a product decision are named.
- [x] Edge cases cover hostile packages, hostile plugins, slow PCs, memory pressure, sleep and language changes.
- [ ] The manifest format (TOML or JSON) is settled by measuring the `toml` size cost (task T003).
- [ ] The Windows timings of `research.md` are confirmed on a real Windows machine (task T060).
