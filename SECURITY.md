# Security policy

## Supported versions

Only the latest release of Bientôt receives security fixes. The app updates
itself when installed with `Setup.exe` or from the Microsoft Store; a portable
ZIP has to be replaced by hand.

## Report a vulnerability

Do not open a public issue. Report it privately through GitHub:
[open a security advisory](https://github.com/Thanaen/WorkTime/security/advisories/new)
(the **Report a vulnerability** button under the **Security** tab).

Include what an attacker can do, the steps to reproduce it, and the Bientôt
version (Settings › General › About). You can write in English or in French.

Bientôt is maintained by one person in their free time. You should get a
first answer within a week. Once a fix ships, the advisory is published and
credits you unless you ask otherwise.

## Scope

In scope:

- the Windows app (`worktime.exe`) and its installer and update process;
- the web service at `https://bientot.dev`, also reached at
  `https://worktime.thanaen.dev` (`services/worktime`): update redirects,
  downloads and the anonymous usage signal.

What Bientôt stores and sends is listed in [docs/privacy.md](docs/privacy.md).
