#!/usr/bin/env bash
# Where almanac's vendored kp-themes files come from, and how to read
# them back. Sourced by scripts/vendor-kp-themes.sh and by the commit
# gate, so the two cannot disagree about where the header ends.

# vendored path : upstream path, relative to the kp-themes checkout
KP_VENDORED_FILES=(
    "static/themes.css:css/themes.css"
    "static/kp-components.css:css/components.css"
    "static/theme-core.js:js/theme-core.js"
    "static/theme-picker.js:js/theme-picker.js"
    "static/theme-registry.js:js/theme-registry.js"
)

KP_SUMS_FILE="static/KP_THEMES.sha256"

# The upstream content of a vendored file: everything after the blank
# line that closes almanac's provenance header. The header is written by
# one script and read by one gate, and it deliberately contains no blank
# line of its own, so "the first blank line" is an exact boundary rather
# than a pattern that has to be kept in step with upstream's own wording.
kp_upstream_slice() {
    sed '1,/^$/d' "$1"
}
