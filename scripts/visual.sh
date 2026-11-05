#!/usr/bin/env bash
# Visual regression check of the examples on a private virtual display.
#
#   scripts/visual.sh record [scenario…]   capture the baseline (target/visual/baseline)
#   scripts/visual.sh check  [scenario…]   capture again and compare with the baseline
#   scripts/visual.sh list                 print the scenarios
#
# The baseline belongs to the machine: fonts, the GTK version and the renderer
# change pixels. Record it on a commit you trust, change the code, then check.
# A scenario fails when more pixels differ than its tolerance allows; the
# capture and a highlighted difference are left in target/visual/current and
# target/visual/diff.
#
# Environment: PAM_VISUAL_FUZZ (color distance that still counts as equal,
# default 3%), PAM_VISUAL_DIR (default target/visual).
set -uo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
directory=${PAM_VISUAL_DIR:-$root/target/visual}
fuzz=${PAM_VISUAL_FUZZ:-3%}

# name | example | size | seconds | differing pixels allowed | xdotool arguments
# Animated examples are captured after their motion settles; the ones that
# never settle (spinners, video, a clock) get a tolerance instead.
scenarios() {
    cat <<'EOF'
counter|counter|520x560|3|0|
counter-pressed|counter|520x560|3|0|mousemove 76 141 click 1 sleep 0.3 click 1 sleep 0.3 click 1 sleep 0.5 mousemove 400 500
controls|controls|960x640|4|4000|
lists|lists|960x640|4|0|
lists-scrolled|lists|960x640|4|0|mousemove 480 320 click 5 click 5 click 5 click 5 click 5 click 5 sleep 1
chat|chat|960x640|4|0|
navigation|navigation|960x640|4|0|
navigation-photo|navigation|960x640|4|0|mousemove 68 334 click 1 sleep 1.5
navigation-invoice|navigation|960x640|4|0|mousemove 130 430 click 1 sleep 1.5
motion|motion|960x640|5|60000|
gestures|gestures|960x640|4|0|
images|images|960x640|6|0|
media|media|960x640|6|80000|
webview|webview|960x640|7|0|
canvas|canvas|960x640|4|0|
shell|shell|960x640|4|0|
windows|windows|960x640|4|0|
dragdrop|dragdrop|960x640|4|0|
text|text|960x640|4|0|
modules|modules|960x640|4|4000|
links|links|960x640|4|0|
toast|toast|960x640|4|0|
errors|errors|960x640|4|0|
showcase|showcase|1200x800|5|0|
EOF
}

mode=${1:-}
shift || true
case "$mode" in
record | check) ;;
list)
    scenarios | cut -d'|' -f1
    exit 0
    ;;
*)
    sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac

[ -x "$root/target/debug/pam-desktop" ] || {
    echo "build the host first: cargo build -p pam-desktop-host" >&2
    exit 2
}

wanted=" $* "
failed=0
count=0
mkdir -p "$directory/baseline" "$directory/current" "$directory/diff"

while IFS='|' read -r name example size seconds tolerance actions; do
    [ "$#" -gt 0 ] && [ "${wanted#* "$name" }" = "$wanted" ] && continue
    count=$((count + 1))
    if [ "$mode" = record ]; then
        output=$directory/baseline/$name.png
    else
        output=$directory/current/$name.png
    fi
    rm -f "$output"
    # shellcheck disable=SC2086 # the actions are a list of xdotool words
    if [ -n "$actions" ]; then
        "$root/scripts/shot.sh" "$root/examples/$example" "$output" "$size" "$seconds" -- $actions >/dev/null 2>&1
    else
        "$root/scripts/shot.sh" "$root/examples/$example" "$output" "$size" "$seconds" >/dev/null 2>&1
    fi
    if [ ! -s "$output" ]; then
        echo "FAIL  $name: no capture"
        failed=$((failed + 1))
        continue
    fi
    if [ "$mode" = record ]; then
        echo "saved $name"
        continue
    fi
    baseline=$directory/baseline/$name.png
    if [ ! -s "$baseline" ]; then
        echo "FAIL  $name: no baseline (run: scripts/visual.sh record $name)"
        failed=$((failed + 1))
        continue
    fi
    # `compare` prints the number of differing pixels and exits 1 when any differ.
    different=$(compare -metric AE -fuzz "$fuzz" "$baseline" "$output" "$directory/diff/$name.png" 2>&1)
    different=${different%% *}
    different=${different%.*}
    case "$different" in
    '' | *[!0-9]*)
        echo "FAIL  $name: cannot compare ($different)"
        failed=$((failed + 1))
        ;;
    *)
        if [ "$different" -le "$tolerance" ]; then
            echo "ok    $name ($different)"
            rm -f "$directory/diff/$name.png"
        else
            echo "FAIL  $name: $different pixels differ (allowed $tolerance): $directory/diff/$name.png"
            failed=$((failed + 1))
        fi
        ;;
    esac
done < <(scenarios)

if [ "$count" -eq 0 ]; then
    echo "no scenario matches: $*" >&2
    exit 2
fi
[ "$mode" = check ] && echo "$((count - failed)) of $count scenarios match"
[ "$failed" -eq 0 ]
