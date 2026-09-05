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

# K25: the vendored kp-themes files, checked at two severities.
#
# static/themes.css, static/kp-components.css, the three theme-*.js
# modules and static/strings.js are COPIES of ~/Projects/kp-themes —
# almanac has no npm and no build step, so it cannot be a dependency the
# way it is in JobTracker.
# `scripts/vendor-kp-themes.sh` takes them and records what it took.
#
# Two different things can be wrong with a copy, and they deserve
# different answers (kyu's observation, 2026-09-04, on the first version
# of this gate, which blocked on both):
#
#   * SOMEONE EDITED IT — a change made here is a change that vanishes
#     at the next re-vendor, and nothing else would ever report it.
#     Refused. Detected against the recorded checksums, so it works on
#     CI too, where kp-themes is not on the machine.
#
#   * IT HAS FALLEN BEHIND — kp-themes released and this copy is the
#     previous version. Said out loud, not refused: taking a release is
#     a decision with a moment of its own, and blocking every unrelated
#     almanac commit until someone re-vendors makes one project's
#     release break another project's build.
#
# static/theme-bootstrap.js and static/theme-bridge.css are NOT in this
# list: they are almanac's own and have no upstream to match.
# shellcheck source=scripts/kp-vendored.sh
. scripts/kp-vendored.sh

if [ -f "$KP_SUMS_FILE" ]; then
  edited=""
  for pair in "${KP_VENDORED_FILES[@]}"; do
    dest=${pair%%:*}
    [ -f "$dest" ] || continue
    recorded=$(awk -v f="$dest" '$2 == f {print $1}' "$KP_SUMS_FILE")
    actual=$(kp_upstream_slice "$dest" | sha256sum | cut -d' ' -f1)
    if [ -z "$recorded" ]; then
      edited="$edited\n  $dest (no checksum recorded for it)"
    elif [ "$recorded" != "$actual" ]; then
      edited="$edited\n  $dest"
    fi
  done
  if [ -n "$edited" ]; then
    {
      echo "GATE FAILED — a vendored kp-themes file was edited here:"
      printf '%b\n' "$edited"
      echo
      echo "These are copies of ~/Projects/kp-themes. A change made here is a"
      echo "change that disappears at the next re-vendor, silently. Anything"
      echo "almanac-specific belongs in static/theme-bridge.css or"
      echo "static/theme-bootstrap.js; anything everyone needs belongs upstream."
      echo
      echo "What now: undo the edit, or — if kp-themes released and you meant to"
      echo "take it — run ./scripts/vendor-kp-themes.sh, which re-copies all six"
      echo "and records the new checksums."
    } >&2
    exit 1
  fi

  # The second severity: is this copy still the current release?
  KP="${KP_THEMES_DIR:-$HOME/Projects/kp-themes}"
  if [ -d "$KP" ]; then
    behind=""
    for pair in "${KP_VENDORED_FILES[@]}"; do
      dest=${pair%%:*}
      upstream="$KP/${pair#*:}"
      [ -f "$dest" ] && [ -f "$upstream" ] || continue
      diff -q <(kp_upstream_slice "$dest") "$upstream" >/dev/null || behind="$behind $dest"
    done
    if [ -n "$behind" ]; then
      echo "gates: kp-themes has moved on. A version behind:$behind" >&2
      echo "       Not a failure: taking a release is a decision. When you do," >&2
      echo "       run ./scripts/vendor-kp-themes.sh and check that THEMES in" >&2
      echo "       src/shell/dashboard.rs still matches js/theme-registry.js." >&2
    fi
  else
    # Said out loud rather than passing quietly: on CI the source is not
    # there, so only the "was it edited" half of this ran.
    echo "gates: kp-themes is not on this machine, so the vendored copies were" >&2
    echo "       checked for edits but NOT against the current release." >&2
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
