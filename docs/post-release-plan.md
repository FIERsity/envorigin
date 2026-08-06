# Post-release plan (1.13.0+)

Consolidated list of every change we decided on for after the 08-07
release (1.11.0 + 1.12.0). Ordered by priority. "Post-release" begins
once the 08:20 release lands and Show HN is up.

## A. Code changes — 1.13.0 (the UX release)

1. **Unified entry + auto-detection** (highest priority)
   - `envorigin audit` / `scan` / `explain` with no target detect the
     project type automatically: compose → actions → gitlab → circleci
     → dotenv, and print "detected X format" like actionlint does.
   - Backend-prefixed commands (`envorigin gitlab audit`, …) stay for
     power users and CI pinning.
   - The auto-detection logic already exists and is proven in
     `envorigin-action`'s `target` input — port it into the CLI.
   - Why: the biggest onboarding friction (5 audit entry points, users
     must know their project's format). Matches industry convention
     (gitleaks detect, actionlint).

2. **`scan` accepts a path/directory argument** — `envorigin scan
   ./dir` auto-finds the compose file instead of requiring `-f`.

3. **Redaction hint** — when values are redacted, print a one-line
   hint at the end: "values hidden; pass --show-values to reveal".

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

## D. Docs & metadata (do now if approved; else 1.13.0)

7. **README: 5-minute quick start** — right after Install: install →
   cd project → `envorigin audit` → `envorigin explain` → wire CI,
   with a scenario table ("my .env change did nothing → explain; I
   want a CI gate → Action; team rules → init"). README is currently
   691 lines of reference with no onboarding path; the first example
   uses an abstract variable `P`.

8. **Cargo.toml metadata alignment** — description currently mentions
   only Compose + Actions; keywords miss gitlab/circleci/secrets. Fix
   so 1.12.0 (or 1.13.0) ships correct crates.io metadata.

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
