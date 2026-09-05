#!/usr/bin/env bash
# Re-vendor the kp-themes files almanac serves, and record what was taken.
#
# Almanac has no npm and no build step, so @kp-soft/themes cannot be a
# dependency the way it is in JobTracker. These five files are copies —
# and a copy goes stale silently unless something says so out loud.
#
# Run this after kp-themes releases; it is the whole update. It writes
# each file with almanac's provenance header, then records the checksum
# of the content below that header so the commit gate can tell an edited
# copy (which is a mistake) from a copy that has fallen behind (which is
# a decision).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
# shellcheck source=scripts/kp-vendored.sh
. scripts/kp-vendored.sh

KP=${KP_THEMES_DIR:-$HOME/Projects/kp-themes}
if [ ! -d "$KP" ]; then
    echo "kp-themes is not at $KP — clone it or set KP_THEMES_DIR." >&2
    exit 1
fi

version=$(grep -m1 '"version"' "$KP/package.json" | sed 's/.*"\([0-9][^"]*\)".*/\1/')
commit=$(git -C "$KP" rev-parse --short HEAD)
today=$(date -u +%Y-%m-%d)

# Taken from the tag rather than from whatever the working copy is on:
# a vendored file that matches nobody's release is the worst of both.
if git -C "$KP" rev-parse -q --verify "v$version" >/dev/null; then
    if ! git -C "$KP" diff --quiet "v$version" HEAD -- \
        css/themes.css css/components.css js/theme-core.js js/theme-picker.js js/theme-registry.js js/strings.js; then
        echo "kp-themes' working copy differs from tag v$version for the files almanac takes." >&2
        echo "Check out the tag there first, or release what is on HEAD." >&2
        exit 1
    fi
fi

copy() {
    local src="$1" dest="$2" kind="$3" what="$4"
    {
        if [ "$kind" = "css" ]; then
            printf '/* VENDORED — do not edit here. Source: ~/Projects/kp-themes %s\n' "$what"
            printf '   @kp-soft/themes v%s, commit %s, copied verbatim on %s\n' "$version" "$commit" "$today"
            printf '   by scripts/vendor-kp-themes.sh. Almanac has no npm and no build step,\n'
            printf '   so the shared package cannot be a dependency: this is a copy. The\n'
            printf '   commit gate refuses an edited copy and says so when kp-themes has moved\n'
            printf '   on. Anything almanac-specific belongs in theme-bridge.css or\n'
            printf '   static/theme-bootstrap.js. */\n\n'
        else
            printf '// VENDORED — do not edit here. Source: ~/Projects/kp-themes %s\n' "$what"
            printf '// @kp-soft/themes v%s, commit %s, copied verbatim on %s\n' "$version" "$commit" "$today"
            printf '// by scripts/vendor-kp-themes.sh. The commit gate refuses an edited copy\n'
            printf '// and says so when kp-themes has moved on.\n\n'
        fi
        cat "$src"
    } > "$dest"
}

for pair in "${KP_VENDORED_FILES[@]}"; do
    dest=${pair%%:*}
    upstream=${pair#*:}
    case "$dest" in
        *.css) kind=css ;;
        *) kind=js ;;
    esac
    copy "$KP/$upstream" "$dest" "$kind" "$upstream"
done

# What was taken, so an edited copy is detectable without kp-themes
# being on the machine — CI included.
{
    echo "# @kp-soft/themes v$version, commit $commit, vendored $today."
    echo "# sha256 of each file's upstream content: everything after the blank"
    echo "# line that closes almanac's provenance header."
    for pair in "${KP_VENDORED_FILES[@]}"; do
        dest=${pair%%:*}
        printf '%s  %s\n' "$(kp_upstream_slice "$dest" | sha256sum | cut -d' ' -f1)" "$dest"
    done
} > "$KP_SUMS_FILE"

echo "Vendored @kp-soft/themes v$version (commit $commit)."
echo "Check src/shell/dashboard.rs: THEMES must still match js/theme-registry.js."
