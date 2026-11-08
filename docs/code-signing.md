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

Each release also lists the attestation as `Bientot.intoto.jsonl`. With it, the
check does not read GitHub's attestation store:

```bash
gh attestation verify Bientot-Setup.exe --repo Thanaen/Bientot --bundle Bientot.intoto.jsonl
```

An attestation is not a code signature: Windows and SmartScreen do not read it.

## Plugin keys

Plugins carry their own signatures, unrelated to the signature of Bientôt.
Each key is an ed25519 key pair made with `bientot-plugin keygen`. Its
private half is a file that never leaves the machine of the person who made
it; nothing in this repository or its CI holds one.

- **Author keys.** An author signs a package with `bientot-plugin sign`. The
  first install of a plugin id binds it to the key that signed it, and an
  update must carry a signature by that key or by a new key the old one
  announced. Bientôt shows the key's fingerprint in the consent dialog and in
  **Settings › Plugins**; it does not say who owns the key.
- **The revocation key.** The maintainer keeps one more key offline, on their
  own machine. Only its public half goes into the app, with the fetch of the
  list (spec 003, task T054); until then the app fetches no list. The key signs
  the revocation list served at `https://bientot.dev/plugins/revocations.json`
  with `bientot-plugin sign-revocations`, which refuses a list the app would
  refuse. The app accepts only a list signed by it, not older than the last one
  it kept and not expired. Losing this key means shipping a new public key in
  an app update; a leaked key means the same, at once.

See the [package contract](../specs/003-plugin-runtime/contracts/package.md)
and the [revocation contract](../specs/003-plugin-runtime/contracts/revocations.md).

## If this changes

A certificate, or Azure Artifact Signing through a company, would let the
`Release` workflow sign `worktime.exe` and `Bientot-Setup.exe`. That would
change this page, the download notes in the README, and D-016.
