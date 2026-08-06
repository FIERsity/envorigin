#!/usr/bin/env bash
# usage-stats.sh — one-command adoption metrics for EnvOrigin.
#
# Tracks real usage signals across every public surface:
#   crates.io downloads   — installs via cargo install / Cargo.toml
#   Homebrew analytics    — installs via `brew install envorigin`
#   GitHub traffic        — page views and clones (last 14 days)
#   Release binaries      — prebuilt binary downloads (all releases)
#   Code search           — public repos with `uses: FIERsity/envorigin-action`
#   HN Algolia            — mentions on Hacker News
#
# No telemetry is collected from users — these are all public APIs.
# Read the numbers as a trend, not an exact headcount: CI runs, mirrors,
# and our own dogfood jobs inflate raw counts. The code-search hit count
# is the strongest signal — it means someone actually wired the action
# into their CI. Its hits include our own repos' dogfood jobs.
set -euo pipefail

REPO="FIERsity/envorigin"
ACTION_REPO="FIERsity/envorigin-action"
UA="envorigin-usage-stats"

section() { printf '\n\033[1m%s\033[0m\n' "$1"; }

section "== crates.io (cargo install / Cargo.toml) =="
crates="$(curl -sf -H "User-Agent: $UA" https://crates.io/api/v1/crates/envorigin \
  | jq -r '"total downloads: \(.crate.downloads)   last 90d: \(.crate.recent_downloads)   versions: \(.versions|length)"')"
printf '%s\n' "$crates"
curl -sf -H "User-Agent: $UA" https://crates.io/api/v1/crates/envorigin \
  | jq -r '.versions[0:3][] | "  v\(.num): \(.downloads) downloads"'

section "== Homebrew (brew install envorigin) =="
code="$(curl -s -o /dev/null -w '%{http_code}' https://formulae.brew.sh/api/formula/envorigin.json)"
if [ "$code" = "200" ]; then
  curl -s https://formulae.brew.sh/api/formula/envorigin.json \
    | jq -r '.analytics.install["30d"] as $a | "  30d: \($a["envorigin"] // 0)  90d: \($a["envorigin"] // 0)  365d: \($a["envorigin"] // 0)"'
else
  echo "  formula not on formulae.brew.sh yet (HTTP $code) — appears after the next release"
fi

section "== GitHub traffic (last 14 days) =="
gh api "repos/$REPO/traffic/views"  --jq '"  views: \(.count)  unique visitors: \(.uniques)"'
gh api "repos/$REPO/traffic/clones" --jq '"  clones: \(.count)  unique cloners: \(.uniques)"'

section "== Release binary downloads (all releases) =="
gh api "repos/$REPO/releases" --jq 'reduce .[].assets[]? as $a (0; . + $a.download_count) | "  prebuilt binaries downloaded: \(.)"'

section "== Public repos using the action (uses: FIERsity/envorigin-action) =="
count="$(gh api "search/code?q=FIERsity%2Fenvorigin-action&per_page=1" --jq '.total_count')"
echo "  hits: $count (includes our own dogfood jobs; minus 2-3 for real adoption)"

section "== Hacker News mentions =="
hn="$(curl -sf "https://hn.algolia.com/api/v1/search?query=envorigin" | jq -r '.nbHits')"
echo "  mentions: ${hn:-0}"
