#!/usr/bin/env bash
# M8: one version, not three.
#
# Before this, the binary said 0.1.0, the only git tag said v0.0.1, and
# `make tag-minor` bumped a tag without touching Cargo.toml. A
# self-updater comparing "my version" against "latest release" would
# therefore either never update or update on every poll.
#
# Cargo.toml is the single source. This script fails when a tag exists
# that does not match it, and is run by CI on every push and by the
# release flow before anything is published.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

cargo_version=$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')

if [ -z "$cargo_version" ]; then
    echo "could not read the version from Cargo.toml" >&2
    exit 1
fi

# On a tagged commit the tag must match exactly. Off a tag there is
# nothing to compare against, which is the normal state during
# development.
if tag=$(git describe --exact-match --tags 2>/dev/null); then
    if [ "$tag" != "v$cargo_version" ]; then
        {
            echo "VERSION MISMATCH — the git tag and Cargo.toml disagree."
            echo "  git tag:     $tag"
            echo "  Cargo.toml:  $cargo_version  (expected tag: v$cargo_version)"
            echo
            echo "The self-updater compares its built-in version against the latest release"
            echo "tag; if they can drift, it either never updates or updates on every poll."
            echo "Fix: set Cargo.toml's version to match the tag, or retag."
        } >&2
        exit 1
    fi
    echo "version ok: $tag matches Cargo.toml"
else
    echo "version ok: $cargo_version (no tag on this commit)"
fi
