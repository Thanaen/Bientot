#!/usr/bin/env bash
# Keeps, from the release notes GitHub generates (stdin), the pull requests that change the app:
# those scoped `app`, and those with no scope that change what the app is built from. The pull
# requests scoped `site` or `repo` are left out; every other line is kept as GitHub wrote it.
#
#   gh api repos/<owner>/<repo>/releases/generate-notes … --jq .body |
#       scripts/release-notes.sh <previous-tag or empty> <commit>
set -euo pipefail

previous=$1
target=${2:?usage: release-notes.sh <previous-tag or empty> <commit> < generated-notes}
range=$target
[[ -z $previous ]] || range="$previous..$target"

# What the app is built from, for the pull requests that name no scope.
app_paths=(crates packaging Cargo.toml Cargo.lock rust-toolchain.toml)

# The pull requests of the app, by number: squash-merged, their commit subject ends with (#N).
declare -A app
while IFS= read -r line; do
    sha=${line%% *}
    subject=${line#* }
    [[ $subject =~ \(#([0-9]+)\)$ ]] || continue
    number=${BASH_REMATCH[1]}
    scope=""
    [[ $subject =~ ^[a-z]+\(([a-z-]+)\)!?: ]] && scope=${BASH_REMATCH[1]}
    case $scope in
        app) app[$number]=1 ;;
        "") [[ -z $(git diff-tree --no-commit-id --name-only -r "$sha" -- "${app_paths[@]}") ]] || app[$number]=1 ;;
    esac
done < <(git log --no-merges --format='%H %s' "$range")

kept=0
while IFS= read -r line; do
    if [[ $line =~ /pull/([0-9]+)$ ]]; then
        [[ -n ${app[${BASH_REMATCH[1]}]:-} ]] || continue
        kept=$((kept + 1))
    elif [[ $line =~ ^\*\*Full\ Changelog\*\* && $kept == 0 ]]; then
        printf 'No change to the app.\n\n'
        kept=-1
    fi
    printf '%s\n' "$line"
done
