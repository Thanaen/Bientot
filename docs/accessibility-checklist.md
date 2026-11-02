# Accessibility checklist

This page records the keyboard, screen-reader, contrast and non-colour state
checks of v1.0.0 (T109). The run is the one of
[validation-report.md](validation-report.md#run): 2 November 2026, on the
maintainer's PC, with `0.2.0-alpha.90`, the Store build `0.2.62.0`, and the
fixes of [#121](https://github.com/Thanaen/Bientot/pull/121) built locally.

## Screen readers

Narrator and NVDA read an app through UI Automation (UIA). The run dumped the
UIA tree of each window with `System.Windows.Automation`: control type, name,
keyboard focusability, and the Toggle, Value, SelectionItem, ExpandCollapse
and RangeValue states. Narrator itself was not run.

| Window | Result | What a screen reader gets |
| --- | --- | --- |
| Onboarding | Pass after #121 | Radio buttons « Langue de Windows », « Français », « English » in the group « Langue », with their selection. The target edit is « Objectif quotidien en heures et minutes ». The times are spinners « Heures » and « Minutes » in groups named after the time. The switches have their Toggle state. |
| Flyout | Pass | Status « En retard », status « Départ prévu : 03:13 », the explanation lines, « Temps travaillé : 0 min sur 7 h 00 min », a progress bar, and buttons named with their shortcut (« Mettre en pause, Ctrl+Alt+P »). |
| Command palette | Pass | « Palette de commandes », the edit « Rechercher une commande », and the list « Commandes » whose items carry their selection and shortcut. |
| Settings | Pass after #121 | Tabs per page with their selection, then edits with their value, switches with their Toggle state, and a combo box for the language. |
| Week | Pass | One button per day named with its date, status and totals (« Jeudi 3 septembre, Il manque 10 min, 5 h 50 min comptées sur 6 h 00 min, 1 à régler »). Each segment is named with its kind and times (« À vérifier, 10:00–12:00 (2 h 00 min) »). |
| Startup failure | Pass | Title, explanation, the error, and the buttons « Fermer » and « Exporter le diagnostic local ». |
| Tray icon | Pass | Its tooltip names the state in words: « Départ après la limite · 19 min/7 h 00 min · Départ 03:13 · Selon le travail enregistré · Temps non renseigné ». |

The run found five defects, all fixed in #121 and checked again on its build:

- The section headings of « Paramètres » (« Application », « Détection de
  l’activité », « Connexions à Internet », « À propos »…) had an empty name.
  GPUI names a node from its label, not its text.
- The version in « À propos » had an empty name. AccessKit names a label from
  its value. It now reads « Version 0.2.0 ».
- The lunch range was a group that Tab reaches, with no name. It is now
  « Pause déjeuner ».
- « Voir ce qui est envoyé » offered only the ExpandCollapse pattern, and
  `Expand()` did nothing. It now opens and closes; so do the language list and
  the week bubbles.
- The « Afficher dans l’Explorateur » buttons said they were collapsed. They
  are plain buttons now.

`crates/bientot-app/tests/accessible_names.rs` fails when one of these comes
back.

## Keyboard

| Check | Result | Evidence |
| --- | --- | --- |
| Tab order | Pass | In « Paramètres », Tab moves through the pages and draws the focus ring. Automated: `tests/tab_order.rs`. |
| Edit numbers and times | Pass | Up and Down step the spin fields; typed `H:mm` and Enter commit the durations. Every value persisted ([validation-report.md](validation-report.md#schedule-settings-scroll-and-keyboard-scenario)). |
| Disclosures | Pass | Space opens « Voir ce qui est envoyé ». |
| Commands | Pass | Ctrl+Shift+P opens the palette, typing filters it, Enter runs the command (export, quit). Every flyout button names its shortcut. |

## Contrast

Ratios computed with the WCAG formula from `crates/bientot-app/design/tokens.json`,
translucent backgrounds composited over `surface`. Normal text needs 4.5:1,
focus indicators and the parts of a control 3:1.

| Pair | Dark | Light | Result |
| --- | --- | --- | --- |
| `text` on `surface` | 9.42 | 12.85 | Pass |
| `text-muted` on `surface` / `background` | 5.68 / 4.66 | 5.96 / 5.18 | Pass |
| `accent`, `success`, `warning`, `error` on `surface` | 5.28, 6.22, 7.18, 4.67 | 4.73, 5.05, 4.77, 4.89 | Pass |
| `success`, `warning` on their callout | 5.08, 5.75 | 4.85, 5.12 | Pass |
| `error` (state and callout icon) on its callout | 4.11, now 4.60 | 4.62 | Fixed in #121 |
| `border-focused` on `surface` | 5.28 | 2.83, now 3.11 | Fixed in #121 |
| Selected palette row | 4.61 | 4.64 | Pass |
| `text-placeholder` on `editor` / `surface` | 5.06 / 4.51 | 5.16 / 4.52 | Fixed in #123 |
| `text-disabled` on `editor` | 4.08 | 3.78 | Exempt (disabled controls) |

Theme tests keep the fixed pairs at AA in both themes.

In the light theme, the focus ring of an outlined button inside a tinted
callout reads 2.8 to 3.0:1 against the callout and 3.55:1 against the
button's own fill, which WCAG 1.4.11 accepts.

`text-placeholder` now only fills an empty field (« Aucune limite », the
palette search) and draws the neutral strokes of an enabled control (an
unchecked radio ring, timeline borders, step dots, a scroll thumb). Disabled
controls, which WCAG exempts, use `text-disabled`, the former
`text-placeholder` value, so they still look inactive next to an empty field
([#123](https://github.com/Thanaen/Bientot/issues/123)).

## States that do not rely on colour

| Where | Result | Cue besides colour |
| --- | --- | --- |
| Flyout | Pass | The status says it in words (« Dans les temps », « Proche de la limite », « En retard », « Objectif atteint », « Suivi à vérifier »), with an icon per state. |
| Tray icon | Pass | A symbol per state on the disc, and the tooltip in words. |
| Week timelines | Pass | Hatching for gaps and sleep, a dashed border for « À vérifier », a legend, and an icon before each day's status. |
| Problems to settle | Pass | A count in words (« 2 à régler ») beside each day. |
