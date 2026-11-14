#!/usr/bin/env bash
# Prints the Store listing of packaging/store/listing.json at <commit>, with release notes made of
# the user summaries added to scripts/release-summary/ since <since>, the commit of the last Store
# submission: one `fr:` line and one `en:` line each, as in the GitHub releases
# (scripts/release-notes.sh). With no summary in the range, the file's release notes stay.
# Partner Center takes up to 1,500 characters: the lines that do not fit give way to a last line
# that says there are more.
#
#   scripts/store-release-notes.sh <since or empty> <commit> > listing.json
set -euo pipefail
# Lengths count characters, not bytes.
export LC_ALL=C.UTF-8

since=$1
target=${2:?usage: store-release-notes.sh <since or empty> <commit>}
range=$target
[[ -z $since ]] || range="$since..$target"

limit=1500

# The summaries added in the range, oldest first; one that a later commit removed is left out.
fr=()
en=()
while IFS= read -r path; do
    git cat-file -e "$target:$path" 2>/dev/null || continue
    while IFS= read -r line || [[ -n $line ]]; do
        line=${line%$'\r'}
        case $line in
            "fr: "*) fr+=("${line#fr: }") ;;
            "en: "*) en+=("${line#en: }") ;;
        esac
    done < <(git show "$target:$path")
done < <(git log --reverse --diff-filter=A --name-only --format= "$range" -- ':/scripts/release-summary/*.md' | awk '!seen[$0]++')

# The bulleted lines that fit in the limit, then `more` if some did not.
notes() {
    local more=$1
    shift
    local text="" line
    for line in "$@"; do
        line="• $line"
        if ((${#text} + ${#line} + 1 + ${#more} + 3 > limit)); then
            text+=$'\n'"• $more"
            break
        fi
        text+=${text:+$'\n'}$line
    done
    printf '%s' "$text"
}

listing=$(git show "$target:packaging/store/listing.json")
if ((${#fr[@]} == 0)); then
    printf '%s\n' "$listing"
    exit 0
fi

jq --arg fr "$(notes 'Et d’autres améliorations.' "${fr[@]}")" \
    --arg en "$(notes 'And more improvements.' "${en[@]}")" \
    '.["fr-fr"].ReleaseNotes = $fr | .["en-us"].ReleaseNotes = $en' <<< "$listing"
