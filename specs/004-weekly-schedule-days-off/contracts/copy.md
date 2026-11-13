# Contract: user-facing copy

French is the reference, reviewed and approved by the UX Writer (three
rounds). English is a proposal for the string table, to be reviewed with the
first UI pull request. Durations are produced by the app's formatter
(U+00A0 inside, « 7 h 00 min »); a non-breaking space precedes « : » and the
existing U+202F precedes « ? » as in `strings/table.rs`. `<…>` marks a value.

## Day types

| Use | French | English (proposed) |
| --- | --- | --- |
| Selector, worked | Journée travaillée | Working day |
| Selector, half day | Demi-journée de congé | Half day off |
| Selector, leave | Journée de congé | Day off |
| Selector, holiday (built-in holiday only) | Jour férié | Public holiday |
| Collapsed selector | Demi-journée / Congé / Férié | Half day / Day off / Holiday |
| Selector note | Demi-journée : objectif divisé par deux. Journée de congé : pas d’objectif, rien n’est compté. | Half day: half the target. Day off: no target, nothing is counted. |
| Selector note on a holiday | Journée travaillée ou jour férié : Bientôt retient ce choix pour les années suivantes. | Working day or public holiday: Bientôt remembers this choice for the following years. |

## Week view

| Use | French | English (proposed) |
| --- | --- | --- |
| Row status, leave | Congé | Day off |
| Row status, half day | Demi-journée de congé | Half day off |
| Row status, holiday | Férié · <nom> | Holiday · <name> |
| Row status, non-working | Non ouvré | Non-working |
| Legend swatch | Sans travail prévu | No work planned |
| Subtitle | <durée> comptées sur <durée> · <n> congé(s), <n> férié(s), <n> demi-journée(s) | <duration> counted of <duration> · <n> day(s) off, <n> holiday(s), <n> half day(s) |
| Title | Semaine du <j> au <j> <mois> | Week of <month> <d> to <d> |
| Finished day without time | Aucun temps compté | No time counted |
| Its hint | Si c’était un congé, marquez-le pour retirer ses <durée> de l’objectif de la semaine. | If it was a day off, mark it to take its <duration> off the weekly target. |
| Its action | Marquer comme congé | Mark as day off |
| Holiday explanation | Ce jour est férié (<nom>). Bientôt le compte comme tel, sans objectif. Si vous travaillez ce jour-là, passez-le en journée travaillée : ce choix sera retenu pour les années suivantes. | This day is a public holiday (<name>). Bientôt counts it as one, with no target. If you work that day, make it a working day: this choice will be remembered for the following years. |
| Same, holiday is today | Si vous travaillez aujourd’hui, passez ce jour en journée travaillée : ce choix sera retenu pour les années suivantes. | If you work today, make it a working day: this choice will be remembered for the following years. |
| Button | Passer en journée travaillée | Make it a working day |
| Uncounted activity | <durée> d’activité de ce jour ne sont pas comptées. Elles restent enregistrées. | <duration> of activity on this day is not counted. It stays recorded. |
| Same, exactly 1 min | 1 min d’activité de ce jour n’est pas comptée. Elle reste enregistrée. | (same as above) |
| Just marked as leave (week row, with « Annuler ») | <durée> d’activité de ce jour ne sont plus comptées. Elles restent enregistrées. | <duration> of activity on this day is no longer counted. It stays recorded. |
| Undo | Annuler | Undo |
| Details, credit floor | À l’objectif minimum ou en dessous : le crédit ne s’y applique pas. | At or below the minimum target: credit does not apply. |
| Details, lunch | Pause déjeuner non réservée dans la prévision. | Lunch break not reserved in the forecast. |

## Flyout

| Use | French | English (proposed) |
| --- | --- | --- |
| Leave title / body | Journée de congé / Aucun travail prévu | Day off / No work planned |
| Holiday title / body | Jour férié / <nom> · aucun travail prévu | Public holiday / <name> · no work planned |
| Half-day line | Demi-journée de congé | Half day off |
| Non-working day (existing) | Jour non ouvré | Non-working day |
| Worked time without target | Temps travaillé — | Time worked — |

## Palette

| French | English (proposed) |
| --- | --- |
| Journée : marquer comme congé | Day: mark as day off |
| Journée : marquer comme demi-journée de congé | Day: mark as half day off |
| Journée : marquer comme travaillée | Day: mark as working day |
| Journée : marquer comme jour férié (built-in holidays only) | Day: mark as public holiday |

## Settings › Horaires

| Use | French | English (proposed) |
| --- | --- | --- |
| « Objectif minimum » description (changed) | Le crédit hebdomadaire ne fait jamais descendre l’objectif d’un jour sous ce minimum. Heures et minutes, par exemple 6:00. | Weekly credit never brings a day's target below this minimum. Hours and minutes, for example 6:00. |
| Group | Semaine type | Weekly schedule |
| Row title | Jours ouvrés | Working days |
| Row description | Désactivez les jours où vous ne travaillez pas. Un champ laissé vide reprend l’objectif quotidien. | Turn off the days you do not work. An empty field uses the daily target. |
| Day names | Lundi … Dimanche | Monday … Sunday |
| Placeholder | <durée H:mm> · par défaut | <H:mm> · default |
| Off day | Non ouvré | Non-working |
| Floor hint | À l’objectif minimum ou en dessous : le crédit ne s’y applique pas. | At or below the minimum target: credit does not apply. |
| Total label / value | Objectif de la semaine / <durée> sur <n> jours | Weekly target / <duration> over <n> days |
| Note | Un changement s’applique à partir d’aujourd’hui. Les jours passés gardent leur objectif. | A change applies from today. Past days keep their target. |
| Last working day refused | Gardez au moins un jour ouvré. | Keep at least one working day. |
| Group | Jours fériés | Public holidays |
| Calendar | Calendrier — Aucun / France / France (Alsace-Moselle) | Calendar — None / France / France (Alsace-Moselle) |
| Calendar description | Jours fériés calculés sur cet ordinateur, sans connexion. Un jour férié qui tombe un jour ouvré n’a pas d’objectif. | Holidays computed on this computer, offline. A holiday on a working day has no target. |
| List title | Jours fériés travaillés | Holidays you work |
| List description | Les jours fériés que vous avez passés en journée travaillée. Ils le restent les années suivantes. | Holidays you made working days. They stay so in the following years. |
| Remove / tooltip | Retirer / Ce jour redevient férié. | Remove / This day becomes a holiday again. |
| Empty | Aucun pour l’instant. | None yet. |

« Gardez au moins un jour ouvré. » was not part of the reviewed copy; the UX
Writer reviews it with the settings pull request.

## Onboarding, step « Horaires »

| Use | French | English (proposed) |
| --- | --- | --- |
| Row | Jours ouvrés | Working days |
| Chips | Lun. Mar. Mer. Jeu. Ven. Sam. Dim. | Mon Tue Wed Thu Fri Sat Sun |
| Help | Un objectif différent certains jours ? Il se règle ensuite dans Paramètres › Horaires. | A different target on some days? Set it later in Settings › Schedule. |

## Holiday names

| Id | French | English (proposed) |
| --- | --- | --- |
| `new_year` | Jour de l’An | New Year's Day |
| `good_friday` | Vendredi saint | Good Friday |
| `easter_monday` | Lundi de Pâques | Easter Monday |
| `labour_day` | Fête du Travail | Labour Day |
| `victory_1945` | Victoire 1945 | Victory in Europe Day |
| `ascension` | Ascension | Ascension Day |
| `whit_monday` | Lundi de Pentecôte | Whit Monday |
| `bastille_day` | Fête nationale | Bastille Day |
| `assumption` | Assomption | Assumption Day |
| `all_saints` | Toussaint | All Saints' Day |
| `armistice` | Armistice | Armistice Day |
| `christmas` | Noël | Christmas Day |
| `st_stephen` | Saint-Étienne | St Stephen's Day |
