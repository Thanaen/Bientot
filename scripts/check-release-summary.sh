#!/usr/bin/env bash
# Checks the files that a pull request adds to or changes in scripts/release-summary/: each holds
# one `fr: ` line and one `en: ` line, which open the notes of the next release
# (scripts/release-notes.sh). A pull request titled feat(app), fix(app) or perf(app) changes what
# users get, so it must add one.
#
#   scripts/check-release-summary.sh <pull request title> <base commit> <head commit>
set -euo pipefail

title=${1:?usage: check-release-summary.sh <title> <base> <head>}
base=${2:?usage: check-release-summary.sh <title> <base> <head>}
head=${3:?usage: check-release-summary.sh <title> <base> <head>}

failed=0
fail() {
    echo "::error file=$1::$2"
    failed=1
}

mapfile -t added < <(git diff --name-only --diff-filter=A "$base" "$head" -- scripts/release-summary)
mapfile -t changed < <(git diff --name-only --diff-filter=AM "$base" "$head" -- scripts/release-summary)

for path in "${changed[@]}"; do
    [[ $path == *.md ]] || fail "$path" "A release summary is a .md file."
    fr=0
    en=0
    while IFS= read -r line || [[ -n $line ]]; do
        line=${line%$'\r'}
        case $line in
            "fr: "?*)
                fr=$((fr + 1))
                [[ $line != *"'"* ]] || fail "$path" "Write the French apostrophe as ’, not '."
                ;;
            "en: "?*) en=$((en + 1)) ;;
            "") ;;
            *) fail "$path" "\"$line\" is neither a \"fr: \" line nor an \"en: \" line." ;;
        esac
    done < <(git show "$head:$path")
    ((fr == 1 && en == 1)) || fail "$path" "A release summary holds one \"fr: \" line and one \"en: \" line, found $fr and $en."
done

if [[ $title =~ ^(feat|fix|perf)\(app\)!?: ]] && ((${#added[@]} == 0)); then
    echo "::error::A pull request titled ${BASH_REMATCH[1]}(app) changes what users get: add a file to scripts/release-summary/ with one \"fr: \" line and one \"en: \" line saying what changes for them. See CONTRIBUTING.md, \"Commits and pull requests\"."
    failed=1
fi

exit "$failed"
