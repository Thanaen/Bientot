# Contract: texts that differ on Linux

Every text below goes to the three-reviewer panel (designer critique, UX
writing, screenshot review), which approves the final wording in the pull
request of the task named. The State column says where each variant
stands; the new texts are still proposals unless their Task cell says the
panel approved them. French is the source; English follows it.

Rules:

- A text shown on Linux never says « Windows » or « Microsoft »; a text
  shown on Windows never says « Linux » (FR-019, SC-012).
- A Linux text names the user's **session** or **desktop** (« votre
  session », « votre bureau »), not a distribution or a toolkit. It may
  say « système » (« la langue du système ») where it means the user's
  system settings in general, as Settings › Langue › « Système » does.
- The words « tray », « StatusNotifierItem », « Wayland », « X11 »,
  « D-Bus », « logind », « portal » never appear in the interface. They may
  appear in the diagnostic export.
- The mechanism: a resource may have a `.Linux` variant; the string table
  resolves the variant on Linux and the base on Windows. A test fails when
  a base text contains « Windows » or « Microsoft » and has no variant,
  unless the resource is in the test's short list of texts Linux never
  shows, each with its reason (`SHOWN_ON_WINDOWS_ONLY` in
  `crates/bientot-app/src/strings/tests.rs`).

## Variants of the 17 existing texts

| Resource | Windows, French (unchanged) | Linux, French | Linux, English | Task | State |
| --- | --- | --- | --- | --- | --- |
| `StartupDisabledByUser` | Bientôt est désactivé dans les applications de démarrage. Réactivez-le dans Windows. | *(not shown: the state does not exist on Linux)* | — | T115 | No variant; in the test’s list of texts Linux never shows |
| `SettingsStartupChangeFailed` | Windows n’a pas modifié le démarrage. Vérifiez l’état ci-dessus et réessayez. | Le démarrage automatique n’a pas pu être modifié. Vérifiez l’état et réessayez. | Automatic startup could not be changed. Check the status above and try again. | T115 | Approved by the panel (T120) |
| `SettingsStartupOtherCopyMessage` | Une autre copie de Bientôt démarre aussi avec Windows. Désactivez-la : elle peut s’appeler WorkTime ou Bientôt. | Une autre copie de Bientôt démarre aussi à l’ouverture de session. Désactivez le démarrage automatique dans l’une des deux. | Another copy of Bientôt also starts when you sign in. Turn off automatic startup in one of them. | T115 | Approved by the panel (T120) |
| `NotificationRegistrationFailed` | Les notifications Windows sont indisponibles. Le suivi continue. | Les notifications sont indisponibles sur ce bureau. Le suivi continue. | Notifications are unavailable on this desktop. Workday tracking continues. | T103 | Approved by the panel (T120) |
| `SettingsGeneralDescriptionDemoText` | La langue change après le redémarrage de Bientôt. Le démarrage Windows est indisponible en mode démo. | *(not shown: no code read it)* | — | T120 | Deleted on both platforms (T120): no code read it |
| `SettingsStartupInput.AutomationProperties.Name` | Démarrer Bientôt avec Windows | Démarrer Bientôt à l’ouverture de session | Start Bientôt when you sign in | T115 | Approved by the panel (T120) |
| `SettingsStartupCard.Header` | Démarrage Windows | Démarrage automatique | Automatic startup | T115 | Approved by the panel (T120) |
| `OnboardingActivityWatchApiKeyHelp.Text` | … la clé chiffrée pour votre compte Windows … | *(not shown: the ActivityWatch step is no longer reachable; the resource is deleted on both platforms if T120 confirms it is dead)* | — | T120 | Deleted on both platforms (T120): no code read it |
| `OnboardingLanguageIntro.Text` | Bientôt suit la langue de Windows par défaut. Vous pourrez en changer dans Paramètres. | Bientôt suit la langue du système par défaut. Vous pourrez en changer dans Paramètres. | Bientôt follows your system language by default. You can pick another later in Settings. | T126 | Approved by the panel (T120, T126) |
| `OnboardingLanguageSystemFrench` | Langue de Windows (français) | Langue du système (français) | System language (French) | T126 | Approved by the panel (T120); read since T126 |
| `OnboardingLanguageSystemEnglish` | Langue de Windows (anglais) | Langue du système (anglais) | System language (English) | T126 | Approved by the panel (T120); read since T126 |
| `OnboardingLanguageSystem` | Langue de Windows (anglais à la place) | Langue du système (anglais à la place) | System language (English instead) | T126 | Approved by the panel (T126): shown when the system language is neither French nor English |
| `OnboardingStartupTitle.Text` | Démarrer avec Windows | *(not shown: no code read it)* | — | T115 | Deleted on both platforms (T120): no code read it |
| `OnboardingStartupCard.Header` | Démarrage Windows | *(not shown: no code read it)* | — | T115 | Deleted on both platforms (T120): no code read it |
| `OnboardingStartupInput.AutomationProperties.Name` | Démarrer Bientôt avec Windows | *(not shown: no code read it)* | — | T115 | Deleted on both platforms (T120): no code read it |
| `UpdateAvailableDialogMessage.Text` | Une nouvelle version de Bientôt est disponible dans le Microsoft Store. … | *(not shown: no code read it)* | — | T143 | Deleted on both platforms (T120): no code read it |
| `FlyoutRebuiltReviewText` | Votre session Windows était ouverte avant l’installation de Bientôt. Ce temps est compté provisoirement comme du travail. | Votre session était ouverte avant l’installation de Bientôt, qui ne peut donc pas savoir si l’écran a été verrouillé. Ce temps est compté provisoirement comme du travail. | Your session was open before Bientôt was installed, so Bientôt cannot tell whether the screen was locked. This time counts as work for now. | T094 | Approved by the panel (T120) |

Two more texts name the Microsoft Store and have no variant, because no
Linux build has a Store: `UpdateNotice.Message` and `UsagePingChannelStore`.
They are on the same list of the test.

## New texts

| Id | Where | French | English | Task |
| --- | --- | --- | --- | --- |
| `WindowModeNotice.Title` | Flyout, once, when no tray host exists | Ce bureau n’affiche pas d’icône pour Bientôt | This desktop shows no icon for Bientôt | T064 |
| `WindowModeNotice.Text` | Same | Bientôt reste ouvert dans cette fenêtre. Vous pouvez la réduire : le suivi continue. | Bientôt stays open in this window. You can minimize it; tracking continues. | T064 |
| `WindowModeNotice.Link` | Same | Comment afficher l’icône | How to show the icon | T064 |
| `WindowModeNotice.Dismiss` | Same | Compris | Got it | T064 |
| `ActivityUndetectable.Title` | Flyout, while no idle backend answers | Bientôt ne détecte pas votre activité sur ce bureau | Bientôt cannot detect your activity on this desktop | T075 |
| `ActivityUndetectable.Text` | Same | Rien n’est compté automatiquement. Vous pouvez déclarer votre temps à la main. | Nothing is counted automatically. You can log your time by hand. | T075 |
| `ActivityUndetectable.Action` | Same, a filled button, left out on a day off; it opens « Déclarer du temps travaillé » as the week's « Déclarer du temps travaillé… » does | Déclarer du temps | Log time | T075 |
| `ActivityUndetectable.ForecastPaused` | Flyout, same state, the headline of a paused day in place of `ShellForecastAwaitsSource`; the running day's « Prévision indisponible » shows no `FlyoutDegradedHint` then | Pas de prévision sans détection de l’activité | No forecast without activity detection | T075 |
| `LockStateUnknown.Text` | Settings › Diagnostic, under `DiagnosticSection.Locks` while its value is `DiagnosticSection.LocksNotReported`; the export says `lockReporting: unavailable`, since it holds no free text | Ce bureau ne signale pas le verrouillage de l’écran. Bientôt se fie à votre activité. | This desktop does not report screen locks. Bientôt relies on your activity. | T084 |
| `DiagnosticSection.Header` | Settings › Général, the group under « À propos », Linux only | Diagnostic | Diagnostics | T075 |
| `DiagnosticSection.Detection` | Same, the title of the first row, with the idle backend's name as its value | Méthode de détection | Detection method | T075 |
| `DiagnosticSection.ActivityNone` | Same, the row's value while no idle backend answers | Aucune | None | T075 |
| `DiagnosticSection.ActivityNoneText` | Same, under the row's title with that value | Rien n’est compté automatiquement. | Nothing is counted automatically. | T075 |
| `DiagnosticSection.Locks` | Same, the title of the second row, shown once lock reporting is known | Verrouillage de l’écran | Screen lock | T084 |
| `DiagnosticSection.LocksReported` | Same, its value while the desktop reports locks | Signalé | Reported | T084 |
| `DiagnosticSection.LocksNotReported` | Same, its value otherwise, with `LockStateUnknown.Text` under the title | Non signalé | Not reported | T084 |
| `PowerHistoryPartial.Text` | Day details, on the unknown span after a start without a readable journal | L’ordinateur a été en veille à un moment que Bientôt ne peut pas connaître. | The computer slept at a time Bientôt cannot know. | T094 |
| `StartupMismatchAppImage.Text` | Settings › Startup, state `RegistrationMoved`: on an AppImage, the entry starts another absolute path; an info callout in place of the status line, as « Applications de démarrage » on Windows. The onboarding shows it as the status line, without « Corriger », only while its switch is off: « Commencer » with the switch on writes the entry again. The Flatpak's entry keeps `StartupRegistrationMismatch`, already shown on both platforms | L’entrée de démarrage pointe vers un autre emplacement. Le fichier a peut-être été déplacé. | The startup entry points to another location. The file may have been moved. | T115 |
| `StartupMismatch.Action` | Same, the callout's outlined button; it turns the switch on, which writes the entry again for this file | Corriger | Fix | T115 |
| `StartupPortalReason` | The desktop’s own permission prompt (Flatpak) | Bientôt démarre à l’ouverture de session pour compter votre journée dès le début. | Bientôt starts when you sign in so that your day is counted from its beginning. | T112 |
| `LauncherEntry.Header` | Settings › General, « Application », the row between « Langue » and « Démarrage automatique », AppImage only; Windows shows no such row | Menu des applications | App menu | T144 |
| `LauncherEntry.Description` | Same, under the title | Afficher Bientôt dans le menu de votre bureau, pour l’ouvrir sans chercher le fichier AppImage. | Show Bientôt in your desktop’s menu so you can open it without looking for the AppImage file. | T144 |
| `LauncherEntry.Input.AutomationProperties.Name` | Same, the accessible name of the row’s switch, on while the menu shows Bientôt | Afficher Bientôt dans le menu des applications | Show Bientôt in the app menu | T144 |
| `LauncherEntry.ChangeFailed` | Same, under the row in the error colour when the files cannot be written or removed; the switch goes back to the menu as it is, and switching again tries again | Le menu des applications n’a pas pu être modifié. Réessayez. | The app menu could not be changed. Try again. | T144 |
| `InstallKind.Header` | Settings › About, a row under the version, for a Flatpak or an AppImage only; the version row is unchanged, and Windows shows no such row | Type d’installation | Installation type | T121 |
| `InstallKind.Flatpak` / `.AppImage` | Same, the row’s value | Flatpak / AppImage | Flatpak / AppImage | T121 |
| `NoGraphics.Text` | Last-resort message when the first window of the process cannot open; Bientôt then quits | Bientôt n’a pas pu ouvrir de fenêtre et s’est fermé. Vérifiez que les pilotes graphiques de votre système sont installés, puis relancez l’application. | Bientôt could not open a window and has closed. Check that your system’s graphics drivers are installed, then restart the app. | T117. Approved by the panel (T117) |

`InstallKind.*` replaces the earlier proposal « Installé depuis Flathub /
Fichier AppImage »: the app can tell that it runs in a Flatpak, not which
remote installed it, and a row titled like « Version » above it, with its
value at the end, reads in one line in both languages. The panel decides
the final wording.

`LauncherEntry.*` is a row of « Application », the other file Bientôt
writes for the desktop beside « Démarrage automatique ». The panel chose a
switch, built and behaving as the one of « Démarrage automatique », so that
the two rows that add a file for the desktop read and act alike: on while
the menu shows Bientôt, off once the entry is gone, which the user turns on
again from there. While the change runs the switch is disabled. A failure
puts the switch back and shows under the row, as those of the actions of
« Données » do: this row saves no setting, so it has no callout and no
« Réessayer », and switching again tries again. The row sits above
« Démarrage automatique », between it and « Langue », because the callout of
a refused startup change shows under the card and must stay beside its own
row.

`WindowModeNotice.*` is a callout of the flyout, above the day and below
the callouts about the day's time (source, appointment, counted, trust), so a
warning about tracking comes first: the info icon, the title in semibold, the
text, then « Comment afficher l’icône » and « Compris » as two outlined
buttons (it records no time, so no filled button). « Comment afficher
l’icône » opens, with the system's URL opener,
`https://bientot.dev/aide/afficher-l-icone` in French and
`https://bientot.dev/en/help/show-the-icon` in English, the help page of
T173 (`shell::icon_help_url`); T173 creates the page at exactly these two
addresses. « Compris » writes `windowModeNoticeDismissed: true` in
`app-state.json`, and the notice never shows again, on this desktop or
another. Windows never shows it: its notification area is always there.

Texts of the site (download section, help page « Afficher l’icône de
Bientôt », FAQ « Mac ou Linux ? ») and of the Flathub listing are
written in their own tasks (T171 to T173, T130) and reviewed by the same
panel.
