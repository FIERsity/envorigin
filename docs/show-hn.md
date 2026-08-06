# Show HN draft — post after the 08-07 08:20 (Beijing) release lands

Timing recommendation: post 2026-08-07 21:00–23:00 Beijing time
(= Friday 09:00–11:00 ET, the devops peak window). Release must be
verified first: crates.io v1.11.0 + v1.12.0, `brew install
FIERsity/tap/envorigin` at 1.12.0, v1.12.0 5-platform binaries live.

---

## Title (max 80 chars, no hype words)

Show HN: Envorigin – explain and audit env vars in Compose, GitHub Actions, GitLab CI, CircleCI

## Post body (plain text — HN renders no markdown)

Env vars are the worst-documented part of any project. Compose files
pull from shell, .env files, env_file layers and environment: overrides;
CI configs stack workflow/job/step env and interpolation on top. When a
value is wrong, nobody can say which layer won. When a secret leaks, it
is usually a variable under a non-sensitive name (DATABASE_URL with
embedded credentials) that no one reviews.

Envorigin is a CLI that answers "where did this value come from" and
"what is wrong with my env setup" across the four config formats in
general use:

  - docker compose: scan/explain/audit/diff/graph
  - GitHub Actions, GitLab CI, CircleCI: same commands
  - 12 audit checks: sensitive values, placeholder values, credentials
    in URLs, private keys, shadowed/dead env lines, unused variables,
    empty values, missing env files, misconfigured rules
  - --fail-on for CI gating, JSON + GitHub annotations output
  - envorigin.toml rules: required/prefix/forbidden/patterns/allowed/
    max_length
  - LSP server + VS Code extension: live diagnostics in the editor
  - GitHub Action wrapper (uses: FIERsity/envorigin-action@v1)

The features were found by real bugs, not invented. Auditing ~50
workflows across outline, cargo, uv, glab, fdroid, influxdb, supabase,
mastodon and act against real production configs turned up: YAML that
serde_yaml rejects but GitLab accepts (multi-document, multiple <<
merge keys, !reference tags), mastodon's gitignored .env.production
that hard-errored analysis, a 1000-line influxdb CircleCI config with
~100 merge keys. Each one became a fixture and a test.

Validation: 136 tests, 51 real-workflow audits with zero panics, docker
compose config parity check on supabase with zero divergence, clippy -D
warnings, cargo audit 0 vulnerabilities, CodeQL 0 findings.

Install:
  brew install FIERsity/tap/envorigin      (v1.12.0, just released)
  cargo install envorigin                  (v1.12.0)
  GitHub Action: FIERsity/envorigin-action@v1
  VS Code: Envorigin (marketplace)

Honest caveats: the project is brand new (this release pair is 1.11.0 +
1.12.0), zero users so far, and I built it alone. MIT licensed.

What I'd like feedback on: which checks matter in your CI, false
positives on real configs, and whether the GitHub Action gate is
useful enough to run on every PR.

## Comment-prep (expect these questions)

Q: How is this different from gitleaks/trufflehog?
A: Those scan git history for leaked secrets. Envorigin parses the CI
config semantically — resolution chain, shadowing, dead code — and
reports problems in the file, on the line, as you write it (LSP).

Q: What about direnv/dotenvx?
A: Local dev workflows. Envorigin analyzes what is already configured —
it's a linter/explainer, not a runner.

Q: Doesn't `docker compose config` do this?
A: It resolves compose env, but won't tell you which layer won and why,
doesn't audit for leaks or shadowing, and doesn't cover Actions/GitLab/
CircleCI at all.

Q: Does it upload my configs?
A: No. Everything runs locally; audit output goes to your terminal/CI.

Q: Why do I need it if my CI is green?
A: Green CI doesn't see shadowed env lines or a DATABASE_URL with
credentials. That is the point of the audit checks.

## Things to verify before posting (checklist)

- [ ] crates.io shows v1.11.0 and v1.12.0 (curl crates.io/api/v1/crates/envorigin)
- [ ] `brew install FIERsity/tap/envorigin` works and reports 1.12.0
- [ ] v1.12.0 GitHub release has all 5 binaries
- [ ] `./scripts/usage-stats.sh` baseline recorded (already done)
