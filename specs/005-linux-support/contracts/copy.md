# Contract: texts that differ on Linux

Every text below is a **proposal** for the three-reviewer panel (designer
critique, UX writing, screenshot review), which approves the final wording
in the pull request of the task named. French is the source; English
follows it.

Rules:

- A text shown on Linux never says « Windows » or « Microsoft »; a text
  shown on Windows never says « Linux » (FR-019, SC-012).
- A Linux text names the user's **session** or **desktop** (« votre
  session », « votre bureau »), not a distribution or a toolkit.
- The words « tray », « StatusNotifierItem », « Wayland », « X11 »,
  « D-Bus », « logind », « portal » never appear in the interface. They may
  appear in the diagnostic export.
- The mechanism: a resource may have a `.Linux` variant; the string table
  resolves the variant on Linux and the base on Windows. A test fails when
  a base text contains « Windows » or « Microsoft » and has no variant.

## Variants of the 17 existing texts

| Resource | Windows, French (unchanged) | Linux, French | Linux, English | Task |
| --- | --- | --- | --- | --- |
| `StartupDisabledByUser` | Bientôt est désactivé dans les applications de démarrage. Réactivez-le dans Windows. | *(not shown: the state does not exist on Linux)* | — | T115 |
| `SettingsStartupChangeFailed` | Windows n’a pas modifié le démarrage. Vérifiez l’état ci-dessus et réessayez. | Le démarrage n’a pas pu être modifié. Vérifiez l’état ci-dessus et réessayez. | Startup could not be changed. Check the status above and try again. | T115 |
| `SettingsStartupOtherCopyMessage` | Une autre copie de Bientôt démarre aussi avec Windows. Désactivez-la : elle peut s’appeler WorkTime ou Bientôt. | Une autre copie de Bientôt démarre aussi à l’ouverture de session. Désactivez-la dans l’une des deux. | Another copy of Bientôt also starts when you sign in. Turn it off in one of them. | T115 |
| `NotificationRegistrationFailed` | Les notifications Windows sont indisponibles. Le suivi continue. | Les notifications sont indisponibles sur ce bureau. Le suivi continue. | Notifications are unavailable on this desktop. Workday tracking continues. | T103 |
| `SettingsGeneralDescriptionDemoText` | La langue change après le redémarrage de Bientôt. Le démarrage Windows est indisponible en mode démo. | La langue change après le redémarrage de Bientôt. Le démarrage à l’ouverture de session est indisponible en mode démo. | Language changes apply after Bientôt restarts. Starting at sign-in is unavailable in demo mode. | T120 |
| `SettingsStartupInput.AutomationProperties.Name` | Démarrer Bientôt avec Windows | Démarrer Bientôt à l’ouverture de session | Start Bientôt when you sign in | T115 |
| `SettingsStartupCard.Header` | Démarrage Windows | Démarrage à l’ouverture de session | Start at sign-in | T115 |
| `OnboardingActivityWatchApiKeyHelp.Text` | … la clé chiffrée pour votre compte Windows … | *(not shown: the ActivityWatch step is no longer reachable; the resource is deleted on both platforms if T120 confirms it is dead)* | — | T120 |
| `OnboardingLanguageIntro.Text` | Bientôt suit la langue de Windows. Vous pourrez la changer dans Paramètres. | Bientôt suit la langue de votre session. Vous pourrez la changer dans Paramètres. | Bientôt follows your session’s language by default. You can change it later in Settings. | T126 |
| `OnboardingLanguageSystemFrench` | Langue de Windows (français) | Langue de la session (français) | Session language (French) | T126 |
| `OnboardingLanguageSystemEnglish` | Langue de Windows (anglais) | Langue de la session (anglais) | Session language (English) | T126 |
| `OnboardingLanguageSystem` | Langue de Windows | Langue de la session | Session language | T126 |
| `OnboardingStartupTitle.Text` | Démarrer avec Windows | Démarrer à l’ouverture de session | Start when you sign in | T115 |
| `OnboardingStartupCard.Header` | Démarrage Windows | Démarrage à l’ouverture de session | Start at sign-in | T115 |
| `OnboardingStartupInput.AutomationProperties.Name` | Démarrer Bientôt avec Windows | Démarrer Bientôt à l’ouverture de session | Start Bientôt when you sign in | T115 |
| `UpdateAvailableDialogMessage.Text` | Une nouvelle version de Bientôt est disponible dans le Microsoft Store. … | *(not shown: the Flatpak has no in-app update; the AppImage uses the installer build’s texts, which name no platform)* | — | T143 |
| `FlyoutRebuiltReviewText` | Votre session Windows était ouverte avant l’installation de Bientôt. Ce temps est compté provisoirement comme du travail. | Votre session était ouverte avant l’installation de Bientôt. Bientôt ne sait pas si l’écran a été verrouillé entre-temps : ce temps est compté provisoirement comme du travail. | Your session was open before Bientôt was installed. Bientôt cannot tell whether the screen was locked in the meantime, so this time counts as work for now. | T094 |

## New texts

| Id | Where | French | English | Task |
| --- | --- | --- | --- | --- |
| `WindowModeNotice.Title` | Flyout, once, when no tray host exists | Ce bureau n’affiche pas d’icône pour Bientôt | This desktop shows no icon for Bientôt | T064 |
| `WindowModeNotice.Text` | Same | Bientôt reste ouvert dans cette fenêtre. Réduisez-la pour continuer : le suivi ne s’arrête pas. | Bientôt stays open in this window. Minimize it to carry on; tracking does not stop. | T064 |
| `WindowModeNotice.Link` | Same | Afficher l’icône | Show the icon | T064 |
| `WindowModeNotice.Dismiss` | Same | Compris | Got it | T064 |
| `ActivityUndetectable.Title` | Flyout, while no idle backend answers | Bientôt ne détecte pas votre activité sur ce bureau | Bientôt cannot detect your activity on this desktop | T075 |
| `ActivityUndetectable.Text` | Same | Rien n’est compté automatiquement. Vous pouvez déclarer votre temps à la main. | Nothing is counted automatically. You can declare your time by hand. | T075 |
| `ActivityUndetectable.Action` | Same | Déclarer du temps | Declare time | T075 |
| `LockStateUnknown.Text` | Settings › Diagnostic, and the export | Ce bureau ne signale pas le verrouillage de l’écran. Bientôt se fie à votre activité. | This desktop does not report screen locks. Bientôt relies on your activity. | T084 |
| `PowerHistoryPartial.Text` | Day details, on the unknown span after a start without a readable journal | L’ordinateur a été en veille à un moment que Bientôt ne peut pas connaître. | The computer slept at a time Bientôt cannot know. | T094 |
| `StartupMismatchAppImage.Text` | Settings › Startup, state « registration mismatch » on an AppImage | Le démarrage pointe vers un autre emplacement de Bientôt. Le fichier a peut-être été déplacé. | Startup points to another location of Bientôt. The file may have been moved. | T115 |
| `StartupMismatch.Action` | Same | Corriger | Fix | T115 |
| `StartupPortalReason` | The desktop’s own permission prompt (Flatpak) | Bientôt démarre à l’ouverture de session pour compter votre journée dès le début. | Bientôt starts when you sign in so that your day is counted from its beginning. | T112 |
| `LauncherEntry.Remove` | Settings › General, AppImage only | Retirer Bientôt du menu des applications | Remove Bientôt from the app menu | T144 |
| `InstallKind.Flatpak` / `.AppImage` | Settings › About | Installé depuis Flathub / Fichier AppImage | Installed from Flathub / AppImage file | T121 |
| `NoGraphics.Text` | Last-resort message when no window can open | Bientôt n’a pas pu ouvrir sa fenêtre : aucun pilote graphique compatible n’a été trouvé. | Bientôt could not open its window: no compatible graphics driver was found. | T117 |

Texts of the site (download section, help page « Afficher l’icône de
Bientôt sur GNOME », FAQ « Mac ou Linux ? ») and of the Flathub listing
are written in their own tasks (T171 to T173, T130) and reviewed by the
same panel.
