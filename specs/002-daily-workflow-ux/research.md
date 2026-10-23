# Design decisions

The existing history workspace is the only correction workspace. A separate
source-problem window would duplicate freshness, dialog sequencing and date state.

The existing indicator already classifies the full-lunch projection against the
preferred departure and warning margin. A second lunch classifier would duplicate
these rules. The presenter supplies conditional localized wording.

TextBox handles H:mm drafts without NumberBox silently substituting its last
numeric value. Existing grouped saves still validate minimum against target.

Explicit interval work and reclassification both override suspension because
issue 65 requests both paths. A general confirmation of a long locked episode
is distinct and preserves suspension.

The architecture comparison preferred candidate A for fewer state owners and
exact UTC prefill. Candidate B's extra problem collection and restriction to
reclassification add complexity and omit the requested interval-add behavior.
