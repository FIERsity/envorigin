# Contributing to EnvOrigin

Thanks for contributing! This guide covers building, testing, and the
contribution workflow. For the design, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Development setup

```sh
cargo build                 # debug build
cargo test                  # unit + integration + LSP tests (136)
cargo clippy --all-targets -- -D warnings   # must be clean
cargo fmt --check           # must be clean
```

Rust 1.86 is pinned in CI (`dtolnay/rust-toolchain@stable`, toolchain 1.86.0).

## How the repo is organized

- `src/` — the Rust crate (see ARCHITECTURE.md for the module map)
- `tests/` — end-to-end CLI tests over `tests/fixtures/`
- `scripts/` — release and validation tooling:
  - `check-release-ready.sh` — the 8-check release gate
  - `validate-real-repos.sh` — real-repository regression sweep (network)
  - `validate-graphs.sh` — mermaid syntax validation (npm, first run only)
  - `release.sh` — the release pipeline (maintainers only)
- `vscode/` — the VS Code extension (version-synced with the CLI)
- `envorigin-action` — the GitHub Action wrapper lives in a separate
  [repository](https://github.com/FIERsity/envorigin-action)

## Reporting bugs

Include:

- the command you ran and its full output
- the config file that triggered it (or a minimal reproducer)
- `envorigin --version` and your OS

Real-world validation has repeatedly found genuine bugs (multi-document
YAML, merge keys, missing env files, GitLab v2 syntax) — a minimal failing
fixture is the fastest way to a fix.

## Submitting changes

1. Create a feature branch (`fix/…`, `feat/…`, `docs/…`).
2. Make the change; add or update tests in `tests/` (fixtures live in
   `tests/fixtures/`).
3. Run the gate locally: `./scripts/check-release-ready.sh` (and
   `--deep` for the network checks if your change touches analyzers).
4. Open a pull request — CI runs fmt/clippy/tests, the action-wrapper
   dogfood job, and CodeQL. CodeRabbit reviews every PR; address its
   findings or explain why they don't apply.
5. Squash-merge once CI is green. Changes are batched into releases
   (crates.io rate-limits rapid versioning, so features accumulate).

## Notes for maintainers

- Releases: `./scripts/check-release-ready.sh --deep`, then
  `./scripts/release.sh <version>` (gate → bump → tag → GitHub release →
  crates.io → Homebrew formula). `release.yml` attaches the 5-platform
  binaries automatically.
- The VS Code extension version is bumped by `release.sh` via
  `npm version` — never edit it by hand.
- New audit checks, new backends, or analyzer changes should extend
  `scripts/validate-real-repos.sh` — the real-repo sweeps are the best
  bug finder this project has.
