# Quickstart: check the feature by hand

Run on Windows with a fresh data folder or the demo scenarios. Defaults: 7 h, minimum 6 h, credit on, debt on.

## US1 — a day without work costs nothing

1. Demo week: Monday 9 h, Tuesday nothing, Wednesday in progress.
2. Week view: Tuesday reads « Aucun temps compté » in a neutral tone;
   Wednesday's target is 6 h (1 h of credit applied); « Détails de l'activité
   et du crédit » on Tuesday shows no credit applied.

## US2 — weekly schedule

1. **Paramètres › Horaires**: « Semaine type » shows Monday–Friday on, empty
   fields with « 7:00 · par défaut ».
2. Turn Wednesday off, type 4:00 for Friday. « Objectif de la semaine » reads
   « 25 h 00 min sur 4 jours »; Friday shows the hint « À l'objectif minimum ou en
   dessous : le crédit ne s'y applique pas. ».
3. Type 7:00 on Thursday: the field empties again.
4. Week view: Wednesday is « Non ouvré » with an outlined track; earlier
   days of the week keep their targets.
5. Turn Saturday on: the week has six rows and the title ends on Saturday.
6. Try to turn every day off: the last one refuses with a message.
7. Restart: everything is as left; export then import into another folder:
   same week.

## US3 — leave in the current week

1. Open Thursday, pick « Demi-journée de congé »: target 3 h 30 min, subtitle
   « · 1 demi-journée », departure without lunch reservation, flyout
   « Demi-journée de congé ».
2. Open today with counted time, pick « Journée de congé »: the line « …
   d'activité de ce jour ne sont plus comptées. Elles restent
   enregistrées. » with « Annuler »; press it: back as before.
3. Flyout on a day of leave: « Journée de congé », « Aucun travail prévu »,
   « Temps travaillé — », « Passer en journée travaillée ».
4. Palette: the three « Journée : … » commands act on the focused day.
5. Open a day of last week: no selector.

## US4 — public holidays

1. Set the clock to the week of 9 November 2026, calendar France: Wednesday
   reads « Férié · Armistice », the weekly target is 28 h, no badge, no
   notification.
2. Open Wednesday: the explanation and « Passer en journée travaillée ».
   Press it: Wednesday is a 7 h day, recounted from its start; the settings
   list « Armistice » under « Jours fériés travaillés ».
3. Move to the week of 8 November 2027 (Thursday 11): the Armistice is a
   working day without question.
4. « Retirer » it in the settings: it is a holiday again from the current
   week; closed weeks unchanged.
5. Calendar « France (Alsace-Moselle) »: Good Friday 2027 (26 March) is a
   holiday. Calendar « Aucun »: no holiday at all, the list is kept.
6. Make Wednesday non-working in the schedule: the Armistice on a Wednesday
   shows « Non ouvré » and nothing else.
