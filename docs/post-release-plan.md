# Post-release plan (1.13.0+)

Consolidated list of every change we decided on for after the 08-07
release (1.11.0 + 1.12.0). Ordered by priority. "Post-release" begins
once the 08:20 release lands and Show HN is up.

## A. Code changes — UX release (DONE — pulled into 1.12.0, PR #101)

All of the following shipped in 1.12.0 before the release (approved to
merge without CI during the 2026-08-07 GitHub Actions outage; local
verification: 145 tests, clippy, fmt, real-repo sweep):

1. **Unified entry + auto-detection** — top-level `audit`/`scan`/
   `explain`/`graph` detect the project type (compose → actions →
   gitlab → circleci → dotenv) and dispatch; detected file named in
   the output. Backend-prefixed commands stay for power users and CI
   pinning.
2. **Positional path argument** — `envorigin scan ./dir` /
   `envorigin audit compose.yaml`.
3. **Redaction hint** — human output appends "pass --show-values to
   reveal them".
4. **Friendly no-config error** — `no compose or CI config found in
   <dir>` instead of a raw I/O error.

Remaining post-release: rerun CI on main once GitHub Actions recovers
(the merged PRs never ran CI).

## B. Dependency bumps (post-release only)

4. Merge dependabot PRs #63–66 (all CI green, deliberately deferred):
   toml 1.x, sha2 0.11, tower-lsp 0.20, actions/checkout v7. Revert any
   merge whose CI fails.

## C. Feature cuts — data-driven, after Show HN feedback (2–4 weeks)

5. **Cut the VS Code extension client** (`vscode/` + npm deps + version
   sync + vsce check in release.sh/check-release-ready.sh) — keep the
   `envorigin lsp` server (pure Rust, shares the analyzers, zero
   maintenance; neovim/emacs still get editor integration via LSP).
   Rationale: env config files are low-frequency-edited (LSP value
   density is low), the extension is not on the marketplace (invisible
   to users), and it's the only feature with external deps (npm) and a
   gate check. Cancelled if Show HN / issues show strong demand for a
   VS Code extension.

6. **Cut backends/commands by evidence** — usage-stats.sh baselines +
   HN comments + issues decide. Candidates: circleci (lowest reach),
   graph (lowest frequency). Each cut requires evidence; each keep
   requires nothing.

## D. Docs & metadata (DONE — shipped in 1.12.0)

7. **README: 5-minute quick start** — done, PR #102 (install → cd →
   `envorigin audit` → `envorigin explain` → CI, scenario table).
8. **Cargo.toml metadata** — no change needed: description and
   keywords already cover all four backends since 1.11.0.

## E. Post-release actions (not code)

9. Show HN: post 21:00–23:00 Beijing (Fri 09:00–11:00 ET) using
   docs/show-hn.md; the 20:30 scheduler task verifies the release
   first and reminds. Author must stay online 1–2h to reply.

10. Adoption tracking: run `scripts/usage-stats.sh` weekly; record
    baselines (done: 224 crate downloads, 79 binary downloads, 0 real
    users). First real-user signal = code-search `uses:` hits above
    our own repos, or crate downloads per version > ~15, or a star.

11. Release cadence: batch features into fewer, larger releases
    (crates.io 429 rate-limit lesson).
