#!/usr/bin/env bash
# Lints the workspace for Windows from Linux, so that every `cfg(windows)` line type-checks without
# a Windows machine. Linting links nothing, so a stand-in for MSVC's C compiler, which writes empty
# objects, builds the bundled SQLite of `bientot-store`.
#
# bientot-app is linted with each feature set CI builds on Windows (none, `usage-ping`, `release`
# and `direct`), never with `--all-features`: the `screenshots` feature builds only on Linux.
#
# Needs clang, llvm-lib and llvm-rc (the `clang` and `llvm` packages of Debian and Ubuntu), and the
# x86_64-pc-windows-msvc target that rust-toolchain.toml installs.
set -euo pipefail

cd "$(dirname "$0")/.."
target=x86_64-pc-windows-msvc

# At a fixed path: the C compiler's path is part of what Cargo checks before reusing a build.
compiler="${CARGO_TARGET_DIR:-target}/lint-windows/cl"
mkdir -p "$(dirname "$compiler")"
cat > "$compiler" <<'EOF'
#!/bin/sh
for arg in "$@"; do
  case "$arg" in -Fo*) clang --target=x86_64-pc-windows-msvc -c -x c /dev/null -o "${arg#-Fo}" ;; esac
done
EOF
chmod +x "$compiler"
export "CC_${target//-/_}=$(realpath "$compiler")" "AR_${target//-/_}=llvm-lib"

lint() {
  echo "cargo clippy --target $target $*" >&2
  cargo clippy --locked --target "$target" --all-targets "$@" -- -D warnings
}

lint --workspace
for features in usage-ping release direct; do
  lint -p bientot-app --features "$features"
done
