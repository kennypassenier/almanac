#!/usr/bin/env bash
# Refuses to release from a commit whose CI is not green.
#
# Almanac's CI was red for four days — 2026-08-29 to 2026-09-02, through
# seven releases — and nobody read it. Branch protection on main is
# bypassable and every push used the bypass, so the only thing that
# would have caught it was someone looking. This is that someone.
#
# Deliberately NOT a documentation line. The fault was a check that
# existed and went unread; another readable rule would have the same
# shape as the fault.
#
# Exit codes: 0 green, 1 red, 2 unknown (asked and got no usable answer).
# Unknown is not treated as red: a guard that stops the work every time
# GitHub hiccups or the laptop is offline gets deleted within a month,
# and then there is no guard at all.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# Takes an optional commit so the guard itself can be tested against a
# known-red one without tagging anything (which is how the homelab put
# three fake tags on GitHub while testing theirs).
override=${ALMANAC_ALLOW_RED_CI:-}
sha=$(git rev-parse "${1:-HEAD}")
short=${sha:0:7}

if [ -n "$override" ]; then
    echo "CI check skipped: ALMANAC_ALLOW_RED_CI is set. Releasing $short anyway."
    exit 0
fi

if ! command -v gh >/dev/null; then
    echo "cannot check CI for $short: gh is not installed." >&2
    echo "Install it, or set ALMANAC_ALLOW_RED_CI=1 to release without checking." >&2
    exit 2
fi

# The whole run, not one job: `gates` was green throughout the four red
# days, which is exactly how the red one stayed unread.
runs=$(gh run list --workflow=ci.yml --limit 30 \
        --json headSha,status,conclusion 2>/dev/null || true)

if [ -z "$runs" ] || [ "$runs" = "[]" ]; then
    echo "cannot check CI for $short: GitHub returned nothing." >&2
    echo "Set ALMANAC_ALLOW_RED_CI=1 to release without checking." >&2
    exit 2
fi

state=$(printf '%s' "$runs" | jq -r --arg sha "$sha" '
    map(select(.headSha == $sha))
    | if length == 0 then "none"
      elif any(.status != "completed") then "running"
      elif all(.conclusion == "success") then "green"
      else "red" end')

case "$state" in
    green)
        echo "CI is green on $short."
        ;;
    red)
        echo "CI is RED on $short — refusing to release it." >&2
        echo "Look at it first:  gh run list --limit 3" >&2
        echo "Deliberately releasing anyway: ALMANAC_ALLOW_RED_CI=1 make <target>" >&2
        exit 1
        ;;
    running)
        echo "CI is still running on $short — wait for it, or set ALMANAC_ALLOW_RED_CI=1." >&2
        exit 2
        ;;
    none)
        echo "no CI run found for $short — has it been pushed?" >&2
        echo "Push first, or set ALMANAC_ALLOW_RED_CI=1 to release without checking." >&2
        exit 2
        ;;
esac
