# Contributing

This guide covers the build, the tests, CI, localization, and the commit and
spec conventions. The code layout is in [docs/architecture.md](docs/architecture.md).

## Before you start

- Bientôt is maintained by one person in their free time. Reviews can take a
  few days.
- For anything bigger than a small fix, open an issue first and agree on the
  approach there, so your work is not wasted.
- Bugs and ideas go through the [issue forms](https://github.com/Thanaen/Bientot/issues/new/choose).
  Security problems go through [SECURITY.md](SECURITY.md), never a public issue.
- Issues and pull requests can be written in English or in French. Code,
  comments and commit messages are in English.
- By contributing, you agree to license your work under the
  [MIT license](LICENSE) and to follow the [code of conduct](CODE_OF_CONDUCT.md).

## Prerequisites

- `rustup`. The toolchain pinned in `rust-toolchain.toml` installs on the
  first `cargo` command.
- To run the app: Windows 11 x64. The code builds and its tests run on Linux
  and macOS too, except the Windows-only parts listed below.
- To work on the web service in `services/bientot`: [Bun](https://bun.sh).
- PowerShell 7 (`pwsh`) runs the Windows scripts in `scripts/`.

## Run the tests

Run the same checks as CI from the repository root:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Then check the Windows-only code from Linux: the DPAPI secret store, the Win32 calls of the
Windows integration and the notification-area icon of the app. The bundled SQLite of
`bientot-store`, which `bientot-windows` and `bientot-app` depend on, needs the MSVC C
compiler to build for Windows. Give the build a stand-in C compiler that writes empty
objects, since linting links nothing:

```bash
cat > /tmp/fake-cl.sh <<'EOF'
#!/bin/sh
for arg in "$@"; do
  case "$arg" in -Fo*) clang --target=x86_64-pc-windows-msvc -c -x c /dev/null -o "${arg#-Fo}" ;; esac
done
EOF
chmod +x /tmp/fake-cl.sh
CC_x86_64_pc_windows_msvc=/tmp/fake-cl.sh AR_x86_64_pc_windows_msvc=llvm-lib \
  cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc --locked -- -D warnings
```

Its Windows tests run only on Windows.

The first version of the app, then called WorkTime, was a .NET app, removed in October
2026. The Rust app reads the same data files, so several tests replay what the .NET code
computed or wrote:
`crates/bientot-domain/tests/dotnet_fixtures.rs` (the domain), the `bientot-store` tests
(settings files, stored dates and instants, the migration SQL, SQLite databases, a
repository scenario) and the `bientot-core` diagnostics tests. Their fixtures are frozen:
if one of these tests fails, the Rust code changed behavior, so do not edit the fixture to
make it pass. A product decision that changes such a case on purpose lists it as a
`Revision` beside the domain test that replays it, under the decision's name.

To look at the flyout without Windows, render every demo scenario to PNG files in one
language and one theme (Linux, with a Vulkan driver such as Mesa's `mesa-vulkan-drivers`):

```bash
cargo run --profile screenshots --features screenshots --bin bientot-screenshots -p bientot-app -- shots/fr-dark --french --dark
```

The Rust app reads its texts from `crates/bientot-app/strings/fr-FR.resw` and
`en-US.resw` through a generated table. After adding or changing a string in both files,
regenerate it and commit `crates/bientot-app/src/strings/table.rs`:

```bash
BIENTOT_UPDATE_STRINGS=1 cargo test -p bientot-app strings
```

To try the app on Windows with scripted activity and temporary data, run
`cargo run -p bientot-app --bin worktime -- --demo`, with `--english`, `--light` or
`--data-dir <folder>` as needed.

For the web service:

```bash
cd services/bientot
bun install --frozen-lockfile
bun test
```

To fuzz what Bientôt reads from disk (`settings.json`, transfer files, and the dates and
instants of the ledger), install nightly Rust and
[cargo-fuzz](https://rust-fuzz.github.io/book/cargo-fuzz.html), then run a target of `fuzz/`
(`codec`, `settings` or `transfer`):

```bash
cp Cargo.lock fuzz/Cargo.lock
fuzz/seed.sh
cd fuzz && cargo +nightly fuzz run settings -- -max_total_time=60
```

A failing input lands in `fuzz/artifacts/<target>/`. Turn it into a regular test of
`bientot-store` once it is fixed.

### What only Windows checks

A green Linux run does not prove these:

- the Windows tests of `bientot-store` and `bientot-windows` (DPAPI, the Run
  key, the last-input query, the single-instance event);
- the release executable and its size, startup time and memory;
- notifications, the tray icon and anything you see on screen.

## CI

`.github/workflows/ci.yml` runs on pull requests and pushes to `main`.
[CI quality gates](docs/ci-quality-gates.md) has the details. In short:

- **Plan** (Ubuntu) decides which jobs a run needs. For a push to `main` it
  looks for a passed pull request run that built the same tree and, if found,
  skips the other jobs.
- **Rust** (Ubuntu) checks formatting, lints and tests the Cargo workspace.
- **GPUI screenshots** (Ubuntu) renders the screenshots when the app crate
  changes, beside **Rust**. **Publish screenshots** pushes them to the
  `refs/ci-screenshots/<pull request>` ref (not a branch, so Vercel ignores it)
  and links them from the job summary.
- **Rust on Windows** tests `bientot-store` and `bientot-windows`, builds the
  release executable and measures its size, startup time and memory. Then
  `scripts/measure-performance.ps1` runs it with the built-in activity source,
  warns above the spec budgets and fails only above 2 % CPU, 200 MiB or 6 s.
  The job runs only when `crates/bientot-app/`, `crates/bientot-windows/`,
  the Windows code of `bientot-store`, the Cargo manifests, the toolchain,
  `scripts/measure-performance.ps1` or `ci.yml` change, and after **Rust**
  passes.

Keep work out of the Windows job unless it needs Windows, and keep new gates
conditional on the files they check.

## Localization

Bientôt ships in French and English. Every user-visible string lives in both
`crates/bientot-app/strings/fr-FR.resw` and
`crates/bientot-app/strings/en-US.resw` (see [Run the tests](#run-the-tests)
for the table they generate). The tests in
`crates/bientot-app/src/strings/tests.rs` fail when a key is missing or empty
in one language, when format placeholders differ, when French punctuation lacks
its no-break space, or when a text uses a straight apostrophe.

## Commits and pull requests

- Use [Conventional Commits](https://www.conventionalcommits.org/) with a
  scope that names the part of the project the change is for:
  - `app`: the Windows app (`crates/`, `packaging/`, its dependencies);
  - `site`: the website and the web service (`services/bientot/`);
  - `repo`: everything else, such as CI, the release workflow and the
    repository's documentation.

  Types: `feat`, `fix`, `perf`, `refactor`, `build`, `ci`, `docs`, `style`,
  `test`, `chore`, `revert`. For example `feat(app): …`, `fix(site): …`,
  `ci(repo): …`. A change to several parts takes the scope of the one a user
  notices.
- The release notes are the ones GitHub generates, without the pull requests
  scoped `site` or `repo` (`scripts/release-notes.sh`).
- To tell users something in the next release only, such as a moved folder,
  add a Markdown file to `scripts/release-announcements/`. The notes of the
  first release whose range holds the commit that adds it start with it.
- Write the subject in the imperative, lower case after the colon, without a
  final period.
- Explain the why in the body. Name the task ID when the commit completes one
  (for example "Completes T111").
- Pull requests are squash-merged, so the pull request title becomes the
  commit subject on `main`, followed by the pull request number, as in
  `feat(app): … (#119)`. The `Pull request title` check fails when the title
  does not follow the convention above.
- Include tests in proportion to the behavioral risk, and update the spec when
  a user-visible rule changes
  ([constitution](.specify/memory/constitution.md), "Development workflow").
- When a task in `tasks.md` is done, tick it and add a dated note on what was
  delivered and what remains unverified.

## Specs

The project follows [GitHub Spec Kit](https://github.com/github/spec-kit).

- `.specify/memory/constitution.md`: project principles. They override plans
  and tasks.
- `specs/<number>-<name>/`: one folder per feature, with `spec.md`, `plan.md`,
  `research.md`, `data-model.md`, `contracts/`, `quickstart.md` and
  `tasks.md`.
  - `001-core-workday-indicator` is the core product.
  - `002-daily-workflow-ux` holds the daily workflow improvements.
- `.specify/feature.json` names the active feature folder.
- `.specify/templates/` and `.specify/scripts/` are the Spec Kit templates and
  PowerShell scripts.
- The Spec Kit skills are in `.claude/skills/` (Claude Code, `/speckit-plan`)
  and `.agents/skills/` (Codex, `$speckit-plan`).
- `docs/` holds the [architecture](docs/architecture.md), the
  [product decisions](docs/product-decisions.md) and the [privacy](docs/privacy.md) page.
- The specs and plans of `001` and `002` were written for the first .NET app.
  Their product rules still hold; their technical plans and task lists are history.
