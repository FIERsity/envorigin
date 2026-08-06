#!/usr/bin/env bash
# Real-repository regression validation.
#
# Clones a few busy open-source repositories (shallow) and runs the current
# binary against their CI configs, asserting the invariants that keep the
# analyzer honest:
#
#   1. every workflow file audits without panicking (exit 0, `--fail-on none`)
#   2. new checks do not spam: `empty-value` fires on none of the real
#      workflows in the sweep
#   3. `explain --debug` traces resolve with correct source lines on real
#      multi-layer workflows
#   4. known-good real findings stay stable (outline CI credentials)
#
# Network required. Run from the repository root:
#
#   ./scripts/validate-real-repos.sh
#
# Use `ENVORIGIN_BIN=/path/to/envorigin` to test a specific binary
# (default: target/release/envorigin, built on the fly).

set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${ENVORIGIN_BIN:-$REPO_DIR/target/release/envorigin}"
WORK="$(mktemp -d /tmp/envorigin-validate.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

if [[ "$BIN" == "$REPO_DIR/target/release/envorigin" ]]; then
  # Always rebuild the default binary: a stale target/release silently
  # tests ancient code.
  echo "==> building release binary"
  (cd "$REPO_DIR" && cargo build --release --quiet)
fi

echo "==> cloning real repositories (shallow)"
for repo in outline/outline rust-lang/cargo astral-sh/uv; do
  name="$(basename "$repo")"
  if [[ ! -d "$WORK/$name" ]]; then
    git clone -q --depth 1 "https://github.com/$repo.git" "$WORK/$name"
  fi
done
# GitLab-hosted repos exercising multi-document YAML and double merge keys.
for repo in "https://gitlab.com/gitlab-org/cli.git glab" "https://gitlab.com/fdroid/fdroidserver.git fdroid"; do
  read -r url name <<< "$repo"
  if [[ ! -d "$WORK/$name" ]]; then
    git clone -q --depth 1 "$url" "$WORK/$name"
  fi
done
# CircleCI: influxdb's 1000-line config uses ~100 merge keys and executors.
if [[ ! -d "$WORK/influxdb" ]]; then
  git clone -q --depth 1 "https://github.com/influxdata/influxdb.git" "$WORK/influxdb"
fi

failures=0

echo "==> sweep: every workflow audits without panicking"
total=0
for f in "$WORK"/uv/.github/workflows/*.yml \
         "$WORK"/cargo/.github/workflows/*.yml \
         "$WORK"/outline/.github/workflows/*.yml; do
  total=$((total + 1))
  if ! "$BIN" actions audit --file "$f" --fail-on none > /dev/null 2>&1; then
    echo "FAIL (panic/error): $f"
    failures=$((failures + 1))
  fi
done
echo "    audited $total workflows"

echo "==> empty-value does not fire on real workflows"
for f in "$WORK"/uv/.github/workflows/*.yml \
         "$WORK"/cargo/.github/workflows/*.yml \
         "$WORK"/outline/.github/workflows/*.yml; do
  if "$BIN" actions audit --file "$f" 2>/dev/null | grep -q "empty-value"; then
    echo "FAIL (empty-value noise): $f"
    failures=$((failures + 1))
  fi
done

echo "==> explain --debug resolves on real multi-layer workflows"
debug_cases=(
  "cargo/.github/workflows/main.yml MDBOOK_VERSION clippy workflow env"
  "cargo/.github/workflows/main.yml CARGO_BUILD_WARNINGS clippy job env"
)
for case in "${debug_cases[@]}"; do
  read -r file variable job layer <<< "$case"
  output="$("$BIN" actions explain "$variable" -j "$job" --debug --file "$WORK/$file" 2>&1)"
  if [[ "$output" != *"resolution trace for $variable"* || "$output" != *"$layer"* ]]; then
    echo "FAIL (debug trace): $file $variable"
    failures=$((failures + 1))
  fi
done

echo "==> known-good real findings stay stable (outline CI)"
outline_audit="$("$BIN" actions audit --file "$WORK/outline/.github/workflows/ci.yml" 2>&1 || true)"
if [[ "$outline_audit" != *"credential-in-url"* || "$outline_audit" != *"sensitive-value"* ]]; then
  echo "FAIL (expected outline findings missing):"
  echo "$outline_audit" | head -6
  failures=$((failures + 1))
fi

echo "==> compose audit on outline production config"
if ! "$BIN" audit --no-docker-check -f "$WORK/outline/docker-compose.yml" \
    --project-directory "$WORK/outline" > /dev/null 2>&1; then
  echo "FAIL (outline compose audit)"
  failures=$((failures + 1))
fi

echo "==> gitlab backend on real GitLab-hosted configs"
# glab: multi-document YAML (spec doc + pipeline doc), !reference tags,
# anchors; fdroid: two << merge keys per job, include files.
gitlab_cases=(
  "glab/.gitlab-ci.yml GO_VERSION"
  "fdroid/.gitlab-ci.yml GIT_DEPTH"
)
for case in "${gitlab_cases[@]}"; do
  read -r file variable <<< "$case"
  output="$("$BIN" gitlab scan --file "$WORK/$file" 2>&1)"
  if [[ "$output" != *"$variable"* ]]; then
    echo "FAIL (gitlab scan): $file ($variable missing)"
    failures=$((failures + 1))
  fi
done

echo "==> circleci backend on influxdb (1000 lines, ~100 merge keys)"
circleci_output="$("$BIN" circleci scan --file "$WORK/influxdb/.circleci/config.yml" 2>&1)"
if [[ "$circleci_output" != *"job fmt"* || "$circleci_output" != *"CARGO_BUILD_JOBS"* ]]; then
  echo "FAIL (circleci scan): influxdb"
  failures=$((failures + 1))
fi
if ! "$BIN" circleci audit --file "$WORK/influxdb/.circleci/config.yml" > /dev/null 2>&1; then
  echo "FAIL (circleci audit): influxdb"
  failures=$((failures + 1))
fi

if [[ $failures -gt 0 ]]; then
  echo "FAILED: $failures check(s)"
  exit 1
fi
echo "OK — all real-repo regression checks passed"
