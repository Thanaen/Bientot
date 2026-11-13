# Contract: calculation rules per day plan

Example settings: « Objectif quotidien » 7 h, « Objectif minimum » 6 h,
credit and debt on.

| Plan | Target | Credit applied | Debt | Departure | PC time |
| --- | --- | --- | --- | --- | --- |
| Worked (daily target) | 7 h | ≤ 7 h − 6 h = 1 h | against target after credit, if finished with counted work | as today | counted |
| Worked (own target, « jour court » 4 h) | 4 h | 0 (target ≤ minimum) | against 4 h | against 4 h, no lunch reservation | counted |
| Worked (own target 6 h 30) | 6 h 30 | ≤ 30 min | against target after credit | as today, lunch reserved | counted |
| Half day of leave | ⌈target / 2⌉ = 3 h 30 | 0 here (3 h 30 ≤ 6 h) | against 3 h 30 | against 3 h 30, no lunch reservation | counted |
| Leave | none | 0, credit carries on | none | none | not counted, kept |
| Holiday | none | 0, credit carries on | none | none | not counted, kept |
| Non-working | none | 0, credit carries on | none | none | not counted, kept |
| Worked, finished with zero work (US1) | target, shown | 0 | none | n/a | n/a |

Common rules:

1. Surplus beyond the target after credit repays debt first, then becomes
   credit (unchanged).
2. Credit and debt end with the week, Sunday night; unused credit expires
   (D-007 otherwise unchanged).
3. The weekly target is the sum of day targets before credit.
4. « Compté » holds only real work.
5. A change of schedule, daily target or minimum applies from the day it is
   made; earlier days keep their plan and target.

## Reference weeks (tests)

Each row is a test in `crates/bientot-domain/src/credit/tests.rs`; expected
values in minutes.

| Week | Mon | Tue | Wed | Thu | Fri | Expected |
| --- | --- | --- | --- | --- | --- | --- |
| W1 zero day | 540 | 0 (finished) | 360 | – | – | Tue applied 0; Wed applied 60, target 360, no debt |
| W2 holiday | 480 | 10 (Holiday) | 420 | – | – | Tue no target, no debt, 10 uncounted; Wed applied 60 |
| W3 half day | 420 | 210 (HalfDayLeave) | 420 | – | – | Tue target 210, applied 0, debt 0 |
| W4 part-time | 420 | 420 | NonWorking | 420 | 240 own | weekly target 1500; Fri applied 0 |
| W5 Saturday | NonWorking | 480 | 420 | 420 | 420 | Sat 420 working; credit from Tue applies Wed; week target 2100 |
| W6 mid-week change | 420 (7 h) | 420 (7 h) | change daily target to 6 h | 360 | 360 | Mon, Tue keep 420; Wed–Fri 360; minimum ≤ 360 enforced |
| W7 all remaining off | 540 | 420 | Leave | Leave | Leave | 120 credit expires Sunday |
| W8 defaults unchanged | any | any | any | any | any | identical to the pre-feature results (property test) |
