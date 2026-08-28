#!/usr/bin/env bash
# Signs a release manifest with the offline minisign key (AR19 as
# amended).
#
# Run by Kenny on his own machine, never in CI: a checksum served from
# the same host as the binary proves nothing, so the signature is the
# only thing standing between the updater and a compromised release
# host. CI publishes; Kenny signs.
#
# Usage, from a clean checkout at the tag being released:
#   ./scripts/sign-release.sh target/release/almanac
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

binary=${1:-target/release/almanac}

if [ ! -f "$binary" ]; then
    echo "no binary at $binary — build it first with: cargo build --release" >&2
    exit 1
fi

if ! command -v minisign >/dev/null; then
    echo "minisign is not installed — on Arch: sudo pacman -S minisign" >&2
    exit 1
fi

./scripts/check-version.sh

version=$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
outdir="dist/v$version"
mkdir -p "$outdir"

cp "$binary" "$outdir/almanac"

# The manifest is what gets signed: the updater checks the signature
# over this file, then checks the binary against the hash inside it.
# Signing the hash rather than the binary keeps the signed artifact
# small and lets a future release carry several files.
(cd "$outdir" && sha256sum almanac > SHA256SUMS)

# How a running Almanac discovers that a newer version exists: it
# fetches this one asset from GitHub's "latest release" URL. Deliberately
# a plain file rather than the GitHub API — no token, no rate limit, and
# nothing to parse that an attacker could confuse.
echo "$version" > "$outdir/VERSION"

echo
echo "Signing $outdir/SHA256SUMS — minisign will ask for your key's password."
minisign -Sm "$outdir/SHA256SUMS"

echo
echo "Signed. Attach all four to the GitHub release for v$version:"
ls -1 "$outdir"
echo
echo "  gh release create v$version $outdir/* --title v$version --generate-notes"
echo
echo "Until VERSION is attached to the *latest* release, no running"
echo "instance will see this version at all."
echo
echo "The public key baked into the binary must match ~/.minisign/minisign.pub."
echo "If you ever regenerate the key, update RELEASE_PUBKEY in"
echo "src/shell/update.rs and rebuild — see docs/OPERATIONS_RUNBOOK.md."
