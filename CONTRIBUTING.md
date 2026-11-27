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
- To run the app: Windows 11 x64, or Linux with an X11 or Wayland session.
  The Linux build is not released, and it records no activity.
  [Platforms](docs/architecture.md#platforms) in the architecture says what
  it does. The code also builds and its tests run on macOS, except the
  Windows-only and Linux-only parts listed below.
- On Debian or Ubuntu, the C compiler and the libraries the build links, then
  a Vulkan driver to draw (Mesa's software one works without a GPU). The
  `Rust on Linux` CI job installs the same list:

  ```bash
  sudo apt-get install build-essential pkg-config libfreetype-dev libfontconfig-dev \
    libxkbcommon-dev libxkbcommon-x11-dev mesa-vulkan-drivers
  ```
- To work on the web service in `services/bientot`: [Bun](https://bun.sh).
- PowerShell 7 (`pwsh`) runs the Windows scripts in `scripts/`.

## Run the tests

Run the same checks as CI from the repository root:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

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

Add `--platform linux` to render only the windows whose text or frame differs on Linux, as
Linux shows them; the list is in `crates/bientot-app/src/bin/screenshots/platform.rs`.

The Rust app reads its texts from `crates/bientot-app/strings/fr-FR.resw` and
`en-US.resw` through a generated table. After adding or changing a string in both files,
regenerate it and commit `crates/bientot-app/src/strings/table.rs`:

```bash
BIENTOT_UPDATE_STRINGS=1 cargo test -p bientot-app strings
```

To try the app with scripted activity and temporary data, on Windows or in a Linux desktop
session, run `cargo run -p bientot-app --bin worktime -- --demo`, with `--english`,
`--light` or `--data-dir <folder>` as needed.

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

### Check both platforms

The code of each system lives in its own crate, `bientot-windows` or
`bientot-linux`, and in its backend in `crates/bientot-app/src/platform/`
([architecture](docs/architecture.md#platforms)). A change to code both
systems share needs the checks of both.

On Linux, run the tests that need a desktop session. `cargo test` skips them:
they are marked `#[ignore = "desktop-harness"]` and their names hold
`desktop_harness`. `scripts/linux-desktop-harness.sh` starts a throwaway
session, runs them in it, then stops it. The session has private D-Bus buses,
python-dbusmock's logind and notification server, mocks of a
StatusNotifierWatcher and of Mutter's idle monitor, and a display server:
Xvfb with openbox for `x11`, sway without a screen for `wayland`. Install
what the `Rust` CI job installs, then run both sessions:

```bash
sudo apt-get install xvfb openbox x11-utils xdotool sway wtype grim dbus python3-dbusmock
scripts/linux-desktop-harness.sh x11
scripts/linux-desktop-harness.sh wayland
```

The mocks run on `/usr/bin/python3`. If python-dbusmock is installed for
another interpreter, name it in `HARNESS_PYTHON`. CI also points
`VK_ICD_FILENAMES` at Mesa's software driver (`lvp_icd*.json` in
`/usr/share/vulkan/icd.d/`), so that the app draws without a GPU. A command
after the session name runs in place of the tests, for example the tests of
one crate, or the headless start of a release build that `Rust on Linux`
runs:

```bash
scripts/linux-desktop-harness.sh x11 cargo test -p bientot-linux -- --ignored desktop_harness
scripts/linux-desktop-harness.sh x11 scripts/measure-performance.sh target/release/worktime
```

Then check the Windows-only code from Linux: the DPAPI secret store, the Win32 calls of the
Windows integration and the Windows backend of the app. The `Rust` CI job runs the
same script. It needs `clang` and `llvm` (for `llvm-lib` and `llvm-rc`):

```bash
scripts/lint-windows.sh
```

It lints the workspace for `x86_64-pc-windows-msvc`, then `bientot-app` with each feature set
the Windows job builds (`usage-ping`, `release`, `direct`). It never passes `--all-features`,
whose `screenshots` feature builds only on Linux. The bundled SQLite of `bientot-store` needs
the MSVC C compiler to build for Windows, so the script gives the build a stand-in C compiler
that writes empty objects, since linting links nothing. The Windows tests run only on
Windows.

On Windows, this command runs the Windows tests and the portable half of
`bientot-linux`, as `Rust on Windows` does:

```bash
cargo test --locked -p bientot-platform -p bientot-windows -p bientot-linux
```

No script checks the Linux system calls from Windows. They compile, and
their tests run, only on Linux.

To build the AppImage as `Rust on Linux` does, run this from a clean clone
with its full history, on a Linux host of the target's architecture. The
header of the script lists the tools it needs, such as `vpk` and
`mksquashfs`:

```bash
RELEASE_ANY_BRANCH=1 FIRST_RELEASE=1 scripts/release-direct.sh --target linux-x64 <empty-folder>
```

`RELEASE_ANY_BRANCH=1` builds a commit other than `origin/main`, and
`FIRST_RELEASE=1` skips the delta package, which needs the previous release.
Use `--target linux-arm64` on Arm.

### What only Windows checks

A green Linux run does not prove these:

- the Windows tests of `bientot-store`, `bientot-platform` and
  `bientot-windows` (DPAPI, the Run key, the last-input query, the
  single-instance pipe);
- the release executable and its size, startup time and memory;
- notifications, the tray icon and anything you see on screen.

### What only Linux checks

A green Windows run does not prove these:

- the system calls of `bientot-linux` and the Linux backend of the app,
  which compile only on Linux;
- the desktop harness tests;
- the Linux release executable, the AppImage, the glibc and OpenSSL gates
  and the headless start of `Rust on Linux`.

The harness runs mocks, so a real Linux desktop is still the only check of
what you see on screen: the tray icon, the flyout's place, the
notifications.

## CI

`.github/workflows/ci.yml` runs on pull requests and pushes to `main`.
[CI quality gates](docs/ci-quality-gates.md) has the details. In short:

- **Plan** (Ubuntu) decides which jobs a run needs. For a push to `main` it
  looks for a passed pull request run that built the same tree and, if found,
  skips the other jobs.
- **Rust** (Ubuntu) checks formatting, lints and tests the Cargo workspace,
  runs its desktop tests in headless X11 and Wayland sessions
  (`scripts/linux-desktop-harness.sh`), then lints it for Windows with
  `scripts/lint-windows.sh`.
- **GPUI screenshots** (Ubuntu) renders the screenshots when the app crate
  changes, beside **Rust**. **Publish screenshots** pushes them to the
  `refs/ci-screenshots/<pull request>` ref (not a branch, so Vercel ignores it)
  and links them from the job summary.
- **Rust on Windows** tests `bientot-store`, `bientot-platform`,
  `bientot-windows` and the portable half of `bientot-linux`, builds the
  release executable and measures its size, startup time and memory. Then
  `scripts/measure-performance.ps1` runs it with the built-in activity
  source, warns above the spec budgets and fails only above 2 % CPU, 200 MiB
  or 6 s. The job runs only when the files it builds change (see
  [When each gate runs](docs/ci-quality-gates.md#when-each-gate-runs)), and
  after **Rust** passes.
- **Rust on Linux** runs on Ubuntu 22.04 (glibc 2.35, the oldest the
  AppImage supports). It runs the hostile plugins, builds the Linux release
  executable and the AppImage (`scripts/release-direct.sh --target
  linux-x64`), fails when an executable links OpenSSL or needs a glibc newer
  than 2.35, reports the sizes against 20 000 000 bytes (a warning, not a
  failure, until issue #226 brings them under), and starts the executable,
  then the AppImage, in a headless X11 session
  (`scripts/linux-desktop-harness.sh x11`) with
  `scripts/measure-performance.sh`, which fails when the app is not ready
  within 10 s or writes to standard error, and reports its start time and
  memory. It uploads the executable and the AppImage. It runs on every
  change that runs **Rust**, and on changes to its own three scripts, beside
  **Rust**. **Rust on Linux (aarch64)** runs the same steps on Arm, and
  **Linux screenshot** starts both builds on the newest Ubuntu, whose Mesa
  draws under Xvfb, and uploads a screenshot of the flyout.

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

A key whose text names Windows gets a Linux variant under the same name with
`.Linux` added, such as `SettingsStartupCard.Header.Linux`, in both files: Linux
shows it in place of the base text, and Windows shows the base. The tests fail
when a text Linux shows names Windows or Microsoft, or a text Windows shows
names Linux.

## Commits and pull requests

- Use [Conventional Commits](https://www.conventionalcommits.org/) with a
  scope that names the part of the project the change is for:
  - `app`: the app on either platform (`crates/`, `packaging/`, its
    dependencies);
  - `site`: the website and the web service (`services/bientot/`);
  - `repo`: everything else, such as CI, the release workflow and the
    repository's documentation.

  Types: `feat`, `fix`, `perf`, `refactor`, `build`, `ci`, `docs`, `style`,
  `test`, `chore`, `revert`. For example `feat(app): …`, `fix(site): …`,
  `ci(repo): …`. A change to several parts takes the scope of the one a user
  notices.
- The release notes are the ones GitHub generates, without the pull requests
  scoped `site` or `repo` (`scripts/release-notes.sh`). They open with a
  short summary for users, in French and in English.
- A pull request titled `feat(app)`, `fix(app)` or `perf(app)` adds a file to
  `scripts/release-summary/`, named after the change, with one line in each
  language that says what changes for users:

  ```
  fr: Dans la semaine, la bulle d’une période sélectionnée se ferme d’un clic en dehors ou avec sa croix.
  en: In the week view, the bubble of a selected range closes when you click outside it or on its ×.
  ```

  The `Release summary` check fails without it, and checks the format of every
  file added there. A release with no such file says that nothing visible
  changed.
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
- New features and bug fixes come with automated tests in the same pull
  request, in proportion to the behavioral risk. Update the spec when a
  user-visible rule changes
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
  - `003-plugin-runtime` holds the first step of the plugin system: the
    WebAssembly runtime, one flyout card and palette commands.
  - `004-weekly-schedule-days-off` holds the weekly schedule, leave and
    public holidays.
  - `005-linux-support` holds the Linux build: the platform ports, the Linux
    backends, the packages and their CI gates.
- `.specify/feature.json` names the active feature folder.
- `.specify/templates/` and `.specify/scripts/` are the Spec Kit templates and
  PowerShell scripts.
- The Spec Kit skills are in `.claude/skills/` (Claude Code, `/speckit-plan`)
  and `.agents/skills/` (Codex, `$speckit-plan`).
- `docs/` holds the [architecture](docs/architecture.md), the
  [product decisions](docs/product-decisions.md) and the [privacy](docs/privacy.md) page.
- The specs and plans of `001` and `002` were written for the first .NET app.
  Their product rules still hold; their technical plans and task lists are history.
