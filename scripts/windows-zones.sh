#!/usr/bin/env bash
# Writes crates/bientot-platform/src/zones/windows.rs, the table from the Windows key of each time
# zone to its IANA name, from CLDR's windowsZones.xml at a release tag (release-49 by default).
#
# Each key takes the zone CLDR gives it for the world (territory 001). CLDR names some of those
# zones by an old IANA name, such as Asia/Calcutta, which Ubuntu 24.04 no longer installs without
# tzdata-legacy, so the table holds the current name that common/bcp47/timezone.xml gives it.
#
#   scripts/windows-zones.sh [release tag]
set -euo pipefail

cd "$(dirname "$0")/.."
tag=${1:-release-49}
output=crates/bientot-platform/src/zones/windows.rs
source="https://raw.githubusercontent.com/unicode-org/cldr/$tag/common"

downloads=$(mktemp -d)
trap 'rm -rf "$downloads"' EXIT
curl -sSfL -o "$downloads/windowsZones.xml" "$source/supplemental/windowsZones.xml"
curl -sSfL -o "$downloads/timezone.xml" "$source/bcp47/timezone.xml"

versions=$(grep -o 'otherVersion="[^"]*" typeVersion="[^"]*"' "$downloads/windowsZones.xml")

{
    echo "// Written by scripts/windows-zones.sh from CLDR $tag ($versions)."
    echo "// Do not edit: run the script again with a later release tag."
    echo
    echo "/// Each Windows key and the current IANA name of the zone CLDR gives it for the world."
    echo "pub(super) const WINDOWS_ZONES: &[(&str, &str)] = &["
    awk '
        function attribute(line, name,    found) {
            if (match(line, name "=\"[^\"]*\"")) {
                found = substr(line, RSTART + length(name) + 2, RLENGTH - length(name) - 3)
                return found
            }
            return ""
        }
        FNR == NR {
            if ($0 ~ /<type / && attribute($0, "alias") != "") {
                count = split(attribute($0, "alias"), names, " ")
                current = attribute($0, "iana")
                if (current == "") current = names[1]
                for (i = 1; i <= count; i++) iana[names[i]] = current
            }
            next
        }
        /<mapZone / && attribute($0, "territory") == "001" {
            key = attribute($0, "other")
            zone = attribute($0, "type")
            if (!(zone in iana)) {
                print "no bcp47 entry names " zone > "/dev/stderr"
                exit 1
            }
            printf "    (\"%s\", \"%s\"),\n", key, iana[zone]
        }
    ' "$downloads/timezone.xml" "$downloads/windowsZones.xml" | LC_ALL=C sort
    echo "];"
} > "$output"

rustfmt --edition 2024 "$output"
echo "$output: $(grep -c '^    ("' "$output") Windows keys from CLDR $tag"
