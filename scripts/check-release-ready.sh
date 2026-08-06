#!/usr/bin/env bash
# One-command release readiness gate: every check that must pass before
# scripts/release.sh runs. Mirrors the pre-flight validation used for the
# v1.12.0 release (9 checks).
#
# Usage:
#   ./scripts/check-release-ready.sh            # core gates (no network)
#   ./scripts/check-release-ready.sh --deep     # + real-repo + mermaid checks
#
# Exit 1 on the first failing gate.

set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_DIR"

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

echo "==> 1/8 working tree clean"
[[ -z "$(git status --porcelain)" ]] || fail "working tree is not clean"
echo "    ok ($(git log --oneline -1))"

echo "==> 2/8 tests"
cargo test --quiet || fail "tests"

echo "==> 3/8 clippy (-D warnings)"
cargo clippy --all-targets -- -D warnings || fail "clippy"

echo "==> 4/8 formatting"
cargo fmt --check || fail "fmt"

echo "==> 5/8 release.sh syntax"
bash -n scripts/release.sh || fail "release.sh syntax"

echo "==> 6/8 workflows (actionlint)"
if command -v actionlint > /dev/null 2>&1; then
  actionlint .github/workflows/*.yml || fail "actionlint"
else
  echo "    skipped (actionlint not installed)"
fi

echo "==> 7/8 dependencies (cargo audit, RustSec)"
if command -v cargo-audit > /dev/null 2>&1; then
  cargo audit --quiet || fail "cargo audit"
else
  echo "    skipped (cargo-audit not installed; cargo install cargo-audit)"
fi

echo "==> 8/8 VS Code extension packages"
if command -v npx > /dev/null 2>&1; then
  (cd vscode && npx --yes --cache /tmp/npm-cache-env @vscode/vsce package --no-dependencies --out /tmp/envorigin-ready.vsix > /dev/null 2>&1) \
    || fail "vsce package"
  rm -f /tmp/envorigin-ready.vsix
else
  echo "    skipped (npx not installed)"
fi

if [[ "${1:-}" == "--deep" ]]; then
  echo "==> deep: real-repo regression (network)"
  bash scripts/validate-real-repos.sh || fail "real-repo regression"
  echo "==> deep: mermaid graphs (network)"
  bash scripts/validate-graphs.sh || fail "mermaid graphs"
fi

echo "OK — release ready"
