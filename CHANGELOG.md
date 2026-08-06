# Changelog

All notable changes to EnvOrigin are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and this project adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `diff --format json` for project diffs (`--project-a`/`--project-b`) —
  machine-readable drift reports, sensitive values redacted unless
  `--show-values`.
- `diff --fail-on-drift` — exit 1 when any variable carries different values;
  a CI gate for both dotenv-file and project diff.
- `unknown-rule-variable` audit warning — `[patterns]`/`[allowed]`/
  `[max_length]` keys in `envorigin.toml` that match no variable in the
  audited project are reported (typo in the config would otherwise silently
  disable the rule). Keys also named in `required` are exempt.
- GitHub Releases carry prebuilt binaries for linux-x86_64, macOS
  arm64/x86_64, and Windows x86_64 (built by `release.yml` on every
  published release; re-runnable via `workflow_dispatch`).

## [1.10.0] - 2026-08-06

### Added

- `diff --project-a <dir> --project-b <dir>` — compare two Compose projects'
  final resolved environments (drift per service variable, one-sided
  variables per project; sensitive values redacted unless `--show-values`).

## [1.9.0] - 2026-08-06

### Added

- LSP and the VS Code extension cover dotenv files: `.env` gets live
  security diagnostics in the editor (the same checks as `dotenv audit`).

## [1.8.0] - 2026-08-06

### Added

- `dotenv audit` applies `envorigin.toml` rules (`--config`): prefix,
  forbidden, patterns, allowed, max_length on file entries. Invalid rules
  files fail loudly.

## [1.7.0] - 2026-08-06

### Added

- `envorigin dotenv audit <files...>` — full security check matrix on
  standalone `.env` files with no Compose context, plus `--fail-on` and
  `--format` (including GitHub annotations).

## [1.6.0] - 2026-08-06

### Added

- Audit detects known secret formats by value shape (`AKIA…` AWS,
  `ghp_…` GitHub PAT, `sk_live_…` Stripe, Slack, `sk-…` API keys) —
  more reliable than name matching. All four backends.

## [1.5.0] - 2026-08-06

### Added

- Audit reports `private-key-in-value` (error) for any variable whose value
  embeds a PEM private key block. All four backends.

## [1.4.0] - 2026-08-06

### Added

- Audit reports `credential-in-url` (error) for any variable whose value
  embeds credentials in a URL (`scheme://user:pass@host`) — the most
  common leak vector, usually under non-sensitive names like
  `DATABASE_URL`. All four backends.

## [1.3.0] - 2026-08-06

### Added

- Audit flags unused sensitive interpolation variables: a plaintext secret
  in `.env` gets the full sensitive-value / placeholder /
  secret-manager-reference checks even when no service consumes it.

## [1.2.0] - 2026-08-06

### Added

- `envorigin init` — writes a commented rules template covering all six
  rule types; never overwrites an existing file.

## [1.1.0] - 2026-08-06

### Added

- `audit --ignore <code>` (repeatable) — exempt issue codes across the
  report, annotations, and `--fail-on` exit code, for onboarding legacy
  projects and removing ignores as problems get fixed.

## [1.0.0] - 2026-08-06

### Added

- `explain --debug` — full resolution trace (every definition in the
  interpolation context with precedence and order).

### Changed

- First stable release: the entire roadmap is implemented — four
  backends (Docker Compose, GitHub Actions, GitLab CI, CircleCI) with
  scan/explain/audit/diff/graph, a six-rule conventions engine, LSP +
  VS Code extension, shell completions, JSON/GitHub-annotation audit
  output, and an automated release pipeline (crates.io + Homebrew +
  GitHub Releases), backed by 100 tests and real-repository validation.

## [0.9.0] - 2026-08-06

### Added

- Rules engine: `[allowed]` enum whitelists (`disallowed-value` error) and
  `[max_length]` length caps (`value-too-long` error), alongside
  required/prefix/forbidden/patterns.

## [0.8.0] - 2026-08-06

### Added

- `audit --format github` — GitHub Actions workflow commands; problems are
  annotated on the offending file lines in pull requests
  (severity maps to error/warning/notice; % and newlines escaped).
- CI integration guide in the README (GitHub Actions / GitLab CI /
  CircleCI snippets).

## [0.7.0] - 2026-08-06

### Added

- `--format json` on all four audit commands: machine-readable issues
  array (severity/code/message/path/line) for CI pipelines.

## [0.6.0] - 2026-08-06

### Added

- VS Code extension packaging verified (`npx @vscode/vsce package` +
  `code --install-extension`), version synced with the CLI.
- LSP E2E coverage for all compose filename variants; performance
  documentation (800-variable project scans in ~0.3s).

### Testing

- Unit coverage for graph rendering across all four backends (36 unit
  tests total).

## [0.5.1] - 2026-08-06

### Fixed

- `completions` output piped into `head`/`grep` no longer panics on a
  broken pipe.

## [0.5.0] - 2026-08-06

### Added

- `envorigin completions <shell>` — bash/zsh/fish completion scripts
  (installed automatically by the Homebrew formula).
- Rules engine `[patterns]` — per-variable value format validation
  (`pattern-mismatch` error; invalid regexes reported, not panicked).
- `scripts/release.sh` — one-command release pipeline (verified end to end
  on 0.4.0 and 0.4.1).
- Real-world example in the README (outline production compose audit).

### Fixed

- LSP routed only `.yml` workflow files, silently dropping `.yaml`;
  both suffixes now route. Step-level diagnostics (`GITHUB_ENV` writes)
  now surface in the editor.
- Unnamed action steps displayed the 0-based internal index; now 1-based.

### Testing

- Unit coverage for the Compose, GitLab, and CircleCI analyzers, the
  audit classification predicates, and LSP routing (28 unit tests total).

## [0.4.1] - 2026-08-06

### Fixed

- LSP routed only `.yml` workflow files, silently dropping `.yaml` files
  (no diagnostics or hover in the editor); both suffixes now route.
- LSP now surfaces step-level diagnostics (`GITHUB_ENV` runtime writes were
  invisible in the editor).
- Homebrew formula installs shell completions.

## [0.4.0] - 2026-08-06

### Added

- Rules engine: `[patterns]` per-variable value format validation
  (`pattern-mismatch` error; invalid regexes reported, not panicked).
- `scripts/release.sh` — one-command release (tag, GitHub release,
  crates.io, Homebrew formula update).
- LSP analyzes unsaved buffer edits in memory (all four backends via
  `with_content` variants; relative references still resolve on disk).
- Audit reports `secret-manager-reference` (info) for values pointing at
  Vault, AWS Secrets Manager/SSM, or secret templating.

## [0.3.0] - 2026-08-06

Released to crates.io, GitHub Releases, and the Homebrew tap.

### Added

- `envorigin gitlab` — GitLab CI variables analysis: `include: local` files <
  file-global `variables:` < job-level `variables:`, `$VAR` reference
  tracking, predefined `CI_*`/`GITLAB_*`/`RUNNER_*` variables, external
  `include: remote`/`template` reporting.
- `envorigin circleci` — CircleCI environment analysis: executor
  `environment:` < job `environment:`/`env:` list, `<< parameters.X >>`
  reference tracking to the declaration, `context:`/`<< pipeline.X >>`
  reported as external, `CIRCLE_*` predefined variables.
- `envorigin lsp` — Language Server Protocol server (hover, go-to-definition,
  live diagnostics) plus a VS Code extension client under `vscode/`.
- `envorigin diff` — environment drift comparison across dotenv files
  (sensitive values redacted by default).
- `envorigin audit` / `envorigin actions audit` — env health reports with
  `--fail-on` CI gate: sensitive values (placeholder-aware), shadowed
  dead-code lines, unused interpolation variables.
- `envorigin graph` / `envorigin actions graph` — mermaid provenance
  visualization.
- `COMPOSE_ENV_FILES` expansion; shadowed dead-code annotations in `scan`;
  `env_file format: raw` end-to-end coverage.

### Changed

- Interpolation engine (`InterpolationContext`) is now generic over the
  source-reference type, shared by all four backends.
- Toolchain raised from Rust 1.85 to 1.86 (tower-lsp dependency).
- Audit placeholder detection: example-like values (`WordPress`, `changeit`)
  downgrade to `sensitive-placeholder` warnings instead of errors.

### Fixed

- Interpolation-file variables referenced only through `$VAR`/`${VAR}` were
  reported as unused by the audit; reference detection now scans raw source
  files.
- Real-repository validation against all 39 `docker/awesome-compose` examples
  (all parse and resolve; findings fixed the placeholder and unused-variable
  checks). Real GitLab CI templates and a JSON-format CircleCI config also
  parse and resolve correctly.

## [0.2.0] - 2026-08-06

### Added

- `envorigin actions` — GitHub Actions workflow environment analysis:
  workflow/job/step `env:` layers, `env: file:` references, `${{ }}`
  expression tracking, `GITHUB_ENV` runtime writes flagged, predefined
  variables, `workflow_dispatch`/`workflow_call` inputs.

## [0.1.0] - 2026-08-06

### Added

- `envorigin scan` / `envorigin explain` — Docker Compose environment
  variable provenance: shell, interpolation files, `env_file` layers,
  `environment:` overrides, cross-checked against `docker compose config`.
- Default value redaction (SHA-256 fingerprint), `--show-values`,
  `--format json`, `--host-env-file`, `--no-docker-check`.
