#!/usr/bin/env bash
# Copies the examples out of the repository and installs their dependencies
# against this working tree instead of the released package, so their
# end-to-end scripts test the code under them (Composer does not install a
# path package inside its own source, hence the copy).
#
#   scripts/e2e-setup.sh [example…]   default: every example with tests/e2e.json
#
# PAM_E2E_DIR is where the copies go (default: $RUNNER_TEMP or /tmp, under
# pam-e2e); scripts/e2e.sh reads it too. PAM_E2E_COMPOSER="pam composer"
# where the system PHP is too old.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
target=${PAM_E2E_DIR:-${RUNNER_TEMP:-/tmp}/pam-e2e}
read -r -a composer <<< "${PAM_E2E_COMPOSER:-composer}"
examples=("$@")
if [ ${#examples[@]} -eq 0 ]; then
    for script in "$root"/examples/*/tests/e2e.json; do
        examples+=("$(basename "$(dirname "$(dirname "$script")")")")
    done
fi
mkdir -p "$target"
for example in "${examples[@]}"; do
    directory="$target/$example"
    rm -rf "$directory"
    mkdir -p "$directory"
    (cd "$root/examples/$example" && tar --exclude=./vendor --exclude=./.pam-desktop -cf - .) | (cd "$directory" && tar -xf -)
    # A path repository wins over Packagist; the version satisfies ^2.
    "${composer[@]}" config --working-dir="$directory" repositories.framework \
        "{\"type\": \"path\", \"url\": \"$root\", \"options\": {\"symlink\": true, \"versions\": {\"pushinbr/pam-desktop\": \"2.99.0\"}}}"
    "${composer[@]}" update --working-dir="$directory" pushinbr/pam-desktop --no-interaction --no-progress
    echo "e2e setup $example: $directory, pushinbr/pam-desktop from the working tree"
done
