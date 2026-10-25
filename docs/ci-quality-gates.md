# CI quality gates

The `CI` workflow runs for pull requests and pushes to `main`. It has these jobs:

- `Plan` runs on Ubuntu. It decides which of the jobs below the run needs.
- `Rust` checks formatting, runs clippy with warnings as errors, and runs the tests of the Cargo workspace on Ubuntu. When the app can look different, it also renders the flyout of every demo scenario in French and English, light and dark, with Mesa's software Vulkan driver.
- `Rust on Windows` tests `worktime-store` and `worktime-windows`, builds the release executable of `worktime-app` with its `release` feature (the usage signal), as the Store package and the ZIP ship it, checks that its version resource names it « Bientôt » (`FileDescription` and `ProductName`, which Task Manager and the Startup apps page show), checks that the Velopack build (`direct` feature) compiles, measures the v1 gates (size under 20 MB, ready in under 1 s, under 60 MB of memory) on a throwaway `--data-dir`, so that the run sends no usage signal, builds the portable ZIP and the MSIX from it with `scripts/package-rust.ps1` (each under 20 MB), installs the unsigned MSIX and uninstalls it, and uploads the executable for 7 days. Only the size fails the job: a hosted runner has no GPU, so its time and memory are indications. `scripts/measure-performance.ps1` then measures the spec budgets (SC-007) with the built-in activity source: it warns above them and fails only above 2 % CPU, 200 MiB or 6 s.
- `Record tested tree` records the tree that passed, for the push to `main` that follows the merge.
- `Publish screenshots` pushes the screenshots that `Rust` rendered to the `refs/ci-screenshots/<pull request>` ref, outside the branches so that Vercel does not try to deploy it. The job summary links to each screenshot.

A successful `CI` run requires every job that runs to pass.

## When each gate runs

`Rust` runs for a pull request only when it changes a Rust file (`crates/`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`), `packaging/` (the MSIX manifest and the license texts the tests read) or `.github/workflows/ci.yml`. A pull request that changes only documentation or the web service runs no Rust job.

`Rust on Windows` runs only when a pull request changes `crates/worktime-app/`, `crates/worktime-windows/`, `crates/worktime-store/` (its file handling behaves differently on Windows), `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `packaging/`, `scripts/measure-performance.ps1`, `scripts/package-rust.ps1`, `scripts/verify-package-executables.ps1` or `.github/workflows/ci.yml`, and only after `Rust` passed. Both Rust jobs cache their dependency builds with `Swatinem/rust-cache`; a pull request's cache serves only that pull request's later pushes.

`Rust` renders screenshots, and `Publish screenshots` publishes them, only when a pull request changes `crates/worktime-app/` or `.github/workflows/ci.yml`. Pull requests from forks still render them, but do not publish.

For a pull request, `Record tested tree` runs once every other job has passed or was skipped, and records the tree of the tested merge commit as the `tested-tree-<attempt>` artifact, retained for 30 days. When a push to `main` comes from a merged pull request, `Plan` looks for a successful pull request run of that pull request's last head that recorded the pushed commit's exact tree. When it finds one, the push run skips both Rust jobs, because the same files already passed every gate. When `main` moved after the last pull request run, the trees differ and the push run runs every gate. A direct push, a missing artifact, or an API error also runs every gate.

## Releases

The `Release` workflow (`.github/workflows/release.yml`) publishes the Velopack installer and update packages as a GitHub Release tagged `v<version>` and titled `Bientôt <version>`. It runs every night at 04:17 UTC and does nothing when the commit at the head of `main` already has a release. For an urgent fix, run it from the Actions tab (**Run workflow**). A Windows runner builds the release with `scripts/release-direct.sh`, which computes the delta from the latest release in the feed at `https://worktime.thanaen.dev/updates/`. A separate job, the only one with write access, uploads the files to a draft and publishes it as the latest release once every file is uploaded. Its notes are the ones GitHub generates, from which `scripts/release-notes.sh` removes the pull requests that do not change the app (see CONTRIBUTING.md, "Commits and pull requests"). `scripts/package-rust.ps1` builds the portable ZIP and the MSIX for the Microsoft Store.

## Required checks

Configure `Rust` and `Rust on Windows` as required checks on `main`. A skipped job counts as passed, so a documentation-only pull request is not blocked.
