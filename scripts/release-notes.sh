#!/usr/bin/env bash
# Writes the notes of a release of the app in Markdown: its commits since the previous release,
# grouped by Conventional Commits type. Commits scoped `site` or `repo` are left out, as are
# unscoped ones that change nothing the app is built from.
#
#   scripts/release-notes.sh <previous-tag or empty> <commit>
set -euo pipefail

previous=$1
target=${2:?usage: release-notes.sh <previous-tag or empty> <commit>}
range=$target
[[ -z $previous ]] || range="$previous..$target"

# What the app is built from, for the commits that name no scope.
app_paths=(crates packaging Cargo.toml Cargo.lock rust-toolchain.toml)

declare -A sections
order=(feat fix perf other)
declare -A titles=([feat]="New" [fix]="Fixes" [perf]="Performance" [other]="Other changes")

while IFS= read -r line; do
    sha=${line%% *}
    subject=${line#* }
    if [[ $subject =~ ^([a-z]+)(\(([a-z-]+)\))?!?:\ (.+)$ ]]; then
        type=${BASH_REMATCH[1]} scope=${BASH_REMATCH[3]} text=${BASH_REMATCH[4]}
    else
        type="" scope="" text=$subject
    fi
    case $scope in
        app) ;;
        "") [[ -n $(git diff-tree --root --no-commit-id --name-only -r "$sha" -- "${app_paths[@]}") ]] || continue ;;
        *) continue ;;
    esac
    # What changes nothing a user sees.
    case $type in
        feat | fix | perf) section=$type ;;
        ci | docs | refactor | style | test | chore) continue ;;
        *) section=other ;;
    esac
    sections[$section]+="- ${text^}"$'\n'
done < <(git log --no-merges --format='%H %s' "$range")

printed=false
for section in "${order[@]}"; do
    [[ -n ${sections[$section]:-} ]] || continue
    printf '## %s\n\n%s\n' "${titles[$section]}" "${sections[$section]}"
    printed=true
done
[[ $printed == true ]] || printf 'Maintenance release: nothing changes in the app.\n\n'
if [[ -n $previous && -n ${GITHUB_REPOSITORY:-} ]]; then
    printf '**Full changelog**: https://github.com/%s/compare/%s...%s\n' "$GITHUB_REPOSITORY" "$previous" "$target"
fi
