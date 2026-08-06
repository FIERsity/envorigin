# Show HN draft — post after the 08-07 08:20 (Beijing) release lands

Timing recommendation: post 2026-08-07 21:00–23:00 Beijing time
(= Friday 09:00–11:00 ET, the devops peak window). Release must be
verified first: crates.io v1.11.0 + v1.12.0, `brew install
FIERsity/tap/envorigin` at 1.12.0, v1.12.0 5-platform binaries live.

---

## Title (max 80 chars, no hype words)

Show HN: Envorigin – explain and audit env vars in Compose, GitHub Actions, GitLab CI, CircleCI

## Post body (plain text — HN renders no markdown)

"Why is my .env change not taking effect?" is one of the most-asked
Docker Compose questions on Stack Overflow, and the confusion is real:
Compose reads .env for interpolation but never injects it into the
container; env_file injects but never interpolates; environment:
overrides both without telling you. GitHub Actions stacks workflow /
job / step env and a GITHUB_ENV file that only applies to later steps.
When a value is wrong, nobody can say which layer won. When a secret
leaks, it is usually under a non-sensitive name (DATABASE_URL with
embedded credentials) that no one reviews.

EnvOrigin is a CLI that answers "where did this value come from" and
"what is wrong with my env setup" — one command, format auto-detected
(compose → actions → gitlab → circleci → dotenv):

  envorigin audit      # health report, 12 check types, --fail-on gate
  envorigin scan       # every variable, its source, and the line
  envorigin explain X  # the winner and the shadowed candidates
  envorigin diff       # environment drift across projects/files

No flags to remember: the same commands work in a Compose project, a
GitHub Actions repo, .gitlab-ci.yml, .circleci/config.yml, or a bare
.env directory. Backend-prefixed commands (envorigin gitlab audit)
stay for pinning in CI. Editor integration via LSP (any LSP-capable
editor: Neovim, Emacs, JetBrains) for hover, go-to-definition, and
live diagnostics.

The audit checks were found by real bugs, not invented. Auditing ~50
workflows across outline, cargo, uv, glab, fdroid, influxdb, supabase,
mastodon and act turned up: YAML that serde_yaml rejects but GitLab
accepts (multi-document, multiple << merge keys, !reference tags),
mastodon's gitignored .env.production that hard-errored analysis, a
1000-line influxdb CircleCI config with ~100 merge keys. Each one
became a fixture and a test.

Validation: 145 tests, 52 real-workflow audits with zero panics,
docker compose config parity check on supabase with zero divergence,
clippy -D warnings, cargo audit 0 vulnerabilities, CodeQL 0 findings.

Install:
  brew install FIERsity/tap/envorigin      (v1.12.0, just released)
  cargo install envorigin                  (v1.12.0)
  GitHub Action: FIERsity/envorigin-action@v1 (annotations on PRs)

Honest caveats: the project is brand new (this release pair is 1.11.0 +
1.12.0), zero users so far, and I built it alone. MIT licensed.
Different from gitleaks: that scans git history for leaked secrets —
EnvOrigin parses what your CI config *means* and reports problems in
the file, on the line, as you write it.

What I'd like feedback on: which checks matter in your CI, false
positives on real configs, and whether the GitHub Action gate is
useful enough to run on every PR.

## Comment-prep (expect these questions)

Q: How is this different from gitleaks/trufflehog?
A: Those scan git history for leaked secrets. EnvOrigin parses the CI
config semantically — resolution chain, shadowing, dead code — and
reports problems in the file, on the line, as you write it (LSP).
Complementary: gitleaks keeps secrets out of history, EnvOrigin keeps
configs honest.

Q: Doesn't `docker compose config` do this?
A: It resolves compose env, but won't tell you which layer won and why,
doesn't audit for leaks or shadowing, and doesn't cover Actions/GitLab/
CircleCI at all.

Q: Why does the tool exist if my CI is green?
A: Green CI doesn't see shadowed env lines or a DATABASE_URL with
credentials. That is the point of the audit checks — the same class of
bug as docker/compose#12706 and #12202, which broke env loading in
release builds.

Q: Does it upload my configs?
A: No. Everything runs locally; audit output goes to your terminal/CI.

Q: Is there a VS Code extension?
A: No client extension — `envorigin lsp` is a standard LSP server, so
Neovim, Emacs, JetBrains, and VS Code (any LSP client) can attach.
The extension client was dropped deliberately: env config files are
low-frequency-edited, so a CLI + CI gate covers the real workflow.

Q: Do you handle shell interpolation ($VAR, ${VAR:-default})?
A: Yes — Compose-flavored interpolation with per-reference source
tracking, including derived-from chains (T=${S} → .env:2).

## Things to verify before posting (checklist)

- [ ] crates.io shows v1.11.0 and v1.12.0 (curl crates.io/api/v1/crates/envorigin)
- [ ] `brew install FIERsity/tap/envorigin` works and reports 1.12.0
- [ ] v1.12.0 GitHub release has all 5 binaries
- [ ] `./scripts/usage-stats.sh` baseline recorded (done)
