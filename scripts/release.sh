#!/usr/bin/env bash
# Release automation for EnvOrigin.
#
# Usage: ./scripts/release.sh <version>        # e.g. 0.4.0
#
# 1. verifies the working tree is clean and tests pass
# 2. bumps Cargo.toml + rust-toolchain checks are untouched
# 3. tags and pushes the tag
# 4. creates the GitHub release
# 5. publishes to crates.io
# 6. updates the Homebrew formula (sha256) and pushes the tap
set -euo pipefail

VERSION="${1:?usage: ./scripts/release.sh <version>}"
TAG="v${VERSION}"
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TAP_REPO="${TAP_REPO:-FIERsity/homebrew-envorigin}"
TAP_DIR="${TAP_DIR:-/tmp/homebrew-envorigin-release}"

cd "$REPO_DIR"

if [[ -n "$(git status --porcelain)" ]]; then
  echo "error: working tree is not clean" >&2
  exit 1
fi

echo "==> running tests"
cargo test --quiet
cargo clippy --all-targets -- -D warnings
cargo fmt --check

echo "==> bumping version to $VERSION"
cargo set-version "$VERSION"

git add Cargo.toml Cargo.lock
git commit -m "chore: release v${VERSION}"
git push

echo "==> tagging $TAG"
git tag "$TAG"
git push origin "$TAG"

echo "==> GitHub release"
gh release create "$TAG" --title "EnvOrigin v${VERSION}" --generate-notes

echo "==> crates.io"
cargo publish

echo "==> Homebrew formula"
TARBALL_URL="https://github.com/FIERsity/envorigin/archive/refs/tags/${TAG}.tar.gz"
SHA256="$(curl -sL "$TARBALL_URL" | shasum -a 256 | awk '{print $1}')"
rm -rf "$TAP_DIR"
git clone -q "https://github.com/${TAP_REPO}.git" "$TAP_DIR"
cd "$TAP_DIR"
sed -i '' \
  -e "s|url .*|url \"${TARBALL_URL}\"|" \
  -e "s|sha256 .*|sha256 \"${SHA256}\"|" \
  Formula/envorigin.rb
git add Formula/envorigin.rb
git commit -m "Update envorigin formula v${VERSION}"
git push

echo "==> done: v${VERSION} released to GitHub, crates.io, and Homebrew"
