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
| `Bientot-<version>.msix` | The **Store package** workflow | Nobody | Not installable as is: it only goes to Partner Center, which signs it. The workflow submits it, or it is uploaded by hand. |
| CI MSIX | The `Rust on Windows` job | Nobody | Installed with `-AllowUnsigned` to test the package, then removed. |

The CI MSIX is not the Store package. It wraps the executable the CI job
already built, without the Store channel or version, and adds the
unsigned-package OID to the publisher Partner Center assigned
(`CN=3BB427EF-EC1A-406C-8D5D-D9761E46D4FC`), which lets Windows install it
without a signature. The Store package keeps that publisher as is.

## Where the builds come from

The `Release` workflow builds `Bientot-Setup.exe` and `Bientot-Portable.zip`
on GitHub-hosted runners from a commit on `main`, and publishes them to GitHub
Releases. Each release tag points at the commit it was built from. Nobody builds or
uploads a release by hand.

Before it publishes a release, the workflow attests the build provenance of each
file it uploads. The attestation is signed with a short-lived Sigstore
certificate and stored by GitHub. With the [GitHub CLI](https://cli.github.com/),
this command checks that a workflow of this repository built a downloaded file,
and shows which workflow and commit the attestation names:

```bash
gh attestation verify Bientot-Setup.exe --repo Thanaen/Bientot
```

An attestation is not a code signature: Windows and SmartScreen do not read it.

## If this changes

A certificate, or Azure Artifact Signing through a company, would let the
`Release` workflow sign `worktime.exe` and `Bientot-Setup.exe`. That would
change this page, the download notes in the README, and D-016.
