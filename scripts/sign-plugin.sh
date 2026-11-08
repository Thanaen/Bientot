#!/usr/bin/env bash
# Signs a .bientot-plugin file. An update of a signed plugin installs only when signed by the same
# key, or by a key that key announced, so keep the key file safe and out of the repository.
#
#   scripts/sign-plugin.sh <file.bientot-plugin> --key <key file> [--announce <public key>] [--handover <old key file>]
#   scripts/sign-plugin.sh <file.bientot-plugin> --new-key <key file>   # creates the key, then signs
set -euo pipefail
cd "$(dirname "$0")/.."
package=${1:?usage: sign-plugin.sh <file.bientot-plugin> --key <key file>}
shift
if [[ ${1:-} == --new-key ]]; then
    key=${2:?--new-key needs a key file}
    shift 2
    cargo run --quiet --locked -p bientot-plugin-tool -- keygen "$key"
    set -- --key "$key" "$@"
fi
exec cargo run --quiet --locked -p bientot-plugin-tool -- sign "$package" "$@"
