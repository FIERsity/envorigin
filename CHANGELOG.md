# Changelog

All notable changes to EnvOrigin are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and this project adheres to
[Semantic Versioning](https://semver.org/).

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
