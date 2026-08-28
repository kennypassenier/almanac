#!/usr/bin/env bash
# Almanac quality gates (standing rules 6/7): format, lint with
# warnings as errors, full test suite, and the AR13 module boundary.
# Called by .githooks/pre-commit and .claude/hooks/check-commit.sh;
# non-zero exit blocks the commit.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all

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
