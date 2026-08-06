#!/usr/bin/env bash
# Mermaid syntax regression validation for `envorigin graph`.
#
# mermaid.ink's public API changed and no longer accepts direct POSTs, so
# graph output is validated with the official `mermaid` npm package's
# parser (pure syntax check, no browser needed — puppeteer/chromium are
# NOT installed).
#
# Usage:
#   ./scripts/validate-graphs.sh          # generates graphs from fixtures
#   ./scripts/validate-graphs.sh FILE...  # validates existing .mmd files
#
# Requires node >= 18 and npm (one-time install of mermaid + jsdom into a
# scratch dir; the cache makes repeat runs fast).

set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${ENVORIGIN_BIN:-$REPO_DIR/target/debug/envorigin}"
WORK="$(mktemp -d /tmp/envorigin-mermaid.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

if [[ ! -x "$BIN" ]]; then
  echo "==> building binary"
  (cd "$REPO_DIR" && cargo build --quiet)
fi

if [[ $# -eq 0 ]]; then
  echo "==> generating graphs from fixtures"
  "$BIN" actions graph --file "$REPO_DIR/tests/fixtures/actions/.github/workflows/workflow.yaml" > "$WORK/workflow.mmd"
  "$BIN" gitlab graph --file "$REPO_DIR/tests/fixtures/gitlab/.gitlab-ci.yml" > "$WORK/gitlab.mmd"
  "$BIN" circleci graph --file "$REPO_DIR/tests/fixtures/circleci/.circleci/config.yml" > "$WORK/circleci.mmd" 2>/dev/null || true
  "$BIN" graph --no-docker-check --file "$REPO_DIR/tests/fixtures/merge-keys/compose.yaml" > "$WORK/compose.mmd"
  FILES=("$WORK"/*.mmd)
else
  FILES=("$@")
fi

echo "==> installing mermaid parser (first run only)"
npm install --cache /tmp/npm-cache-env --no-audit --no-fund --prefix "$WORK" mermaid jsdom > /dev/null 2>&1

cat > "$WORK/parse.mjs" <<'EOF'
import { JSDOM } from 'jsdom';
const dom = new JSDOM('<!doctype html><html><body></body></html>');
globalThis.window = dom.window;
globalThis.document = dom.window.document;
Object.defineProperty(globalThis, 'navigator', {
  value: dom.window.navigator,
  configurable: true,
});
const mermaid = (await import('mermaid')).default;
mermaid.initialize({ startOnLoad: false, securityLevel: 'loose' });
import { readFileSync } from 'node:fs';
let failures = 0;
for (const file of process.argv.slice(2)) {
  const text = readFileSync(file, 'utf8');
  try {
    await mermaid.parse(text);
    console.log(`PASS: ${file} (${text.split('\n').length} lines)`);
  } catch (error) {
    console.log(`FAIL: ${file}: ${String(error.message || error).slice(0, 200)}`);
    failures += 1;
  }
}
process.exit(failures ? 1 : 0);
EOF

echo "==> validating mermaid syntax"
node "$WORK/parse.mjs" "${FILES[@]}"
echo "OK — all graphs are valid mermaid"
