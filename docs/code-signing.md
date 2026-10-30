# Code signing

Bientôt has no code-signing certificate. Only the Microsoft Store build is
signed, by Microsoft. This page lists what each build is, who signs it, and
what Windows shows when it is installed.

## Why there is no certificate

The maintainer publishes Bientôt as an individual in France. Azure Artifact
Signing is not open to individuals there. An OV certificate costs money every
year, and a newly signed installer still meets SmartScreen warnings until it
builds a reputation. The Store signs the packages it distributes for free, so
v1 ships through the Store ([D-016](product-decisions.md)).

## Builds

| Build | Where it comes from | Signed by | What Windows shows |
| --- | --- | --- | --- |
| Microsoft Store | Microsoft Store | Microsoft, after certification | Nothing: the Store installs it. |
| `Bientot-Setup.exe` | GitHub Releases, `bientot.dev/download` | Nobody | SmartScreen may ask to confirm with **More info › Run anyway**. |
| `Bientot-Portable.zip` | GitHub Releases | Nobody | SmartScreen may warn the first time `worktime.exe` starts. |
| `Bientot-<version>.msix` | The **Store package** workflow | Nobody | Not installable as is: it is only uploaded to Partner Center, which signs it. |
| CI MSIX | The `Rust on Windows` job | Nobody | Installed with `-AllowUnsigned` to test the package, then removed. |

The Store package and the CI MSIX differ only in the publisher. The Store
package keeps the publisher Partner Center assigned
(`CN=3BB427EF-EC1A-406C-8D5D-D9761E46D4FC`). The CI MSIX adds the
unsigned-package OID to it, which lets Windows install it without a signature.

## Where the builds come from

The `Release` workflow builds `Bientot-Setup.exe` and `Bientot-Portable.zip`
on GitHub-hosted runners from a commit on `main`, and publishes them to GitHub
Releases. Each release tag points at the commit it was built from. Nobody builds or
uploads a release by hand.

## If this changes

A certificate, or Azure Artifact Signing through a company, would let the
`Release` workflow sign `worktime.exe` and `Bientot-Setup.exe`. That would
change this page, the download notes in the README, and D-016.
