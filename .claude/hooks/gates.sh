#!/usr/bin/env bash
# Almanac quality gates (standing rules 6/7): format, lint with
# warnings as errors, full test suite, and the AR13 module boundary.
# Called by .githooks/pre-commit and .claude/hooks/check-commit.sh;
# non-zero exit blocks the commit.
set -euo pipefail

# ── Standing rule 7: a gate that does not predict the build is not a gate ──
# The checks below rewrite files. cargo updates Cargo.lock, formatters
# rewrite sources — and anything rewritten AFTER `git add` is green here
# and absent from the commit. kyu's 1.0.0 commit carried a lock file
# still naming version 0.0.0; the container build refused it one step
# before a release tag, and nothing local had objected. So: fingerprint
# the tree now, compare once the checks are done, and refuse rather than
# report a green run over a tree that moved underneath it.
gate_tree_fingerprint() {
  { git status --porcelain; git diff; } | sha256sum | cut -d' ' -f1
}
gate_tree_before=$(gate_tree_fingerprint)
cd "$(git rev-parse --show-toplevel)"

cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all

# M8: one version. A tag that disagrees with Cargo.toml would make the
# self-updater either never update or update on every poll.
./scripts/check-version.sh >/dev/null

# AR13: the core module must stay free of ambient I/O. The compiler
# cannot enforce a module boundary inside a single crate, so this gate
# does — a hit below means I/O belongs behind a shell-injected trait.
if [ -d src/core ]; then
  if grep -rnE '^[[:space:]]*use[[:space:]]+(reqwest|axum|hyper|tokio::(fs|net|io)|std::(fs|net))' src/core/; then
    echo "GATE FAILED — src/core imports an I/O crate (AR13)." >&2
    echo "Move the I/O behind a trait implemented in the shell module." >&2
    exit 1
  fi
fi

# K25: the vendored kp-themes file must still match its source.
#
# static/themes.css is a COPY of ~/Projects/kp-themes/css/themes.css —
# almanac has no npm and no build step, so it cannot be a dependency the
# way it is in JobTracker. The risk a copy actually runs is not that it
# is wrong today but that it silently ages: upstream releases, nobody
# re-copies, and three projects drift apart on the palettes they claim
# to share.
#
# So: compare, and refuse. Everything above the upstream file's own
# opening comment is almanac's provenance header and is skipped, which
# is why the comparison starts at that line rather than at a line
# number. Taken from kyu, which built and proved this first — including
# the detail that anchoring on a marker rather than a count is what
# keeps it from reporting a difference that is not there.
UPSTREAM_THEMES="$HOME/Projects/kp-themes/css/themes.css"
VENDORED_THEMES="static/themes.css"
if [ -f "$VENDORED_THEMES" ]; then
  if [ -f "$UPSTREAM_THEMES" ]; then
    if ! diff -q \
        <(sed -n '/^\/\* @kp-soft\/themes/,$p' "$VENDORED_THEMES") \
        "$UPSTREAM_THEMES" >/dev/null; then
      {
        echo "GATE FAILED — $VENDORED_THEMES no longer matches kp-themes."
        echo
        echo "It is a vendored copy: either kp-themes released a new version and"
        echo "this copy is stale, or someone edited the copy, which its own header"
        echo "forbids. Anything almanac-specific belongs in theme-bridge.css."
        echo
        echo "The differing lines:"
        diff <(sed -n '/^\/\* @kp-soft\/themes/,$p' "$VENDORED_THEMES") \
             "$UPSTREAM_THEMES" | head -40
        echo
        echo "What now: re-copy the file and bump the version in its header,"
        echo "or move your change into static/theme-bridge.css."
      } >&2
      exit 1
    fi
  else
    # Said out loud rather than passing quietly: on CI the source is not
    # there, and a check that reports success when it did not run is
    # exactly the shape this project spent a day removing.
    echo "gates: kp-themes is not on this machine, so $VENDORED_THEMES was NOT" >&2
    echo "       compared against its source. Checked on Kenny's workstation." >&2
  fi
fi

# Standing rule 7, second clause: see gate_tree_fingerprint above.
if [ "$(gate_tree_fingerprint)" != "$gate_tree_before" ]; then
  {
    echo "gates: the checks rewrote the working tree while they ran."
    echo "A file changed after it was staged, so what this commit carries is"
    echo "NOT what was just tested. Most often this is cargo refreshing"
    echo "Cargo.lock; the changed paths are listed below."
    echo
    git status --porcelain
    echo
    echo "What now: run 'git add -A' and commit again."
  } >&2
  exit 1
fi
