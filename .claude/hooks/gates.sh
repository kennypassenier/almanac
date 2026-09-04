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

# K25: every vendored kp-themes file must still match its source.
#
# static/themes.css, static/kp-components.css and the three theme-*.js
# modules are COPIES of ~/Projects/kp-themes — almanac has no npm and no
# build step, so it cannot be a dependency the way it is in JobTracker.
# The risk a copy actually runs is not that it is wrong today but that
# it silently ages: upstream releases, nobody re-copies, and three
# projects drift apart on the palettes they claim to share. Which is
# exactly what happened between v0.1.1 and v1.0.0 — four new themes that
# almanac's picker could not offer, with nothing failing.
#
# So: compare, and refuse. Each vendored file opens with almanac's own
# provenance header; the comparison starts at the upstream file's first
# line rather than at a line number, which is what keeps this from
# reporting a difference that is not there. Taken from kyu, which built
# and proved the shape first.
#
# static/theme-bootstrap.js is deliberately NOT in this list: it is
# almanac's own glue onto Bootstrap and has no upstream to match.
check_vendored() {
  local vendored="$1" upstream="$2" anchor="$3"
  [ -f "$vendored" ] || return 0
  if [ ! -f "$upstream" ]; then
    # Said out loud rather than passing quietly: on CI the source is not
    # there, and a check that reports success when it did not run is
    # exactly the shape this project spent a day removing.
    echo "gates: kp-themes is not on this machine, so $vendored was NOT" >&2
    echo "       compared against its source. Checked on Kenny's workstation." >&2
    return 0
  fi
  if ! diff -q <(sed -n "/$anchor/,\$p" "$vendored") "$upstream" >/dev/null; then
    {
      echo "GATE FAILED — $vendored no longer matches kp-themes."
      echo
      echo "It is a vendored copy: either kp-themes released a new version and"
      echo "this copy is stale, or someone edited the copy, which its own header"
      echo "forbids. Anything almanac-specific belongs in theme-bridge.css or"
      echo "static/theme-bootstrap.js."
      echo
      echo "The differing lines:"
      diff <(sed -n "/$anchor/,\$p" "$vendored") "$upstream" | head -40
      echo
      echo "What now: re-copy the file and bump the version in its header,"
      echo "and check whether the theme list in src/shell/dashboard.rs still"
      echo "matches the package's registry."
    } >&2
    exit 1
  fi
}

KP="$HOME/Projects/kp-themes"
check_vendored "static/themes.css"        "$KP/css/themes.css"      '^\/\* @kp-soft\/themes'
check_vendored "static/kp-components.css" "$KP/css/components.css"  '^\/\* Component styles'
check_vendored "static/theme-picker.js"   "$KP/js/theme-picker.js"  '^\/\/ The framework-free theme picker'
check_vendored "static/theme-core.js"     "$KP/js/theme-core.js"    '^\/\/ The theme state'
check_vendored "static/theme-registry.js" "$KP/js/theme-registry.js" '^\/\/ GENERATED'

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
