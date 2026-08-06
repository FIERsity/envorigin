# EnvOrigin architecture

How EnvOrigin works under the hood: the module layout, the shared
interpolation engine, the four backends, the audit/rules layers, the LSP
server, and the release pipeline. Written for contributors.

## Crate layout

```
src/
  model.rs          Core types: SourceRef, SourceKind, Candidate, Diagnostic,
                    ProjectReport, ServiceReport, Explanation, AnalysisError
  interpolation.rs  The generic interpolation engine (InterpolationContext<S>)
  yamlx.rs          Tolerant YAML loader: multi-document merging, << merge keys,
                    unknown custom tags (!reference) — everything serde_yaml
                    rejects but real CI files use
  compose.rs        Docker Compose backend (env_file < environment, COMPOSE_ENV_FILES)
  actions.rs        GitHub Actions backend (workflow < job < step env, GITHUB_ENV)
  gitlab.rs         GitLab CI backend (include < global < job, two-pass resolution)
  circleci.rs       CircleCI backend (executor < job, << parameters.X >>)
  audit.rs          Security/convention checks on top of analysis results
  rules.rs          envorigin.toml team-convention rules engine
  diff.rs           Environment drift comparison (dotenv files, Compose projects)
  graph.rs          Mermaid provenance graph rendering (all four backends)
  dotenv.rs         .env parser
  docker.rs         docker compose config --format json canonical verification
  lsp.rs            tower-lsp Language Server (hover, definition, diagnostics)
  output.rs         Human/JSON rendering helpers, fingerprint redaction
  cli.rs            clap definitions for the 12 subcommands
  main.rs           CLI dispatch, exit codes, GitHub annotations
```

## The interpolation engine

`InterpolationContext<S>` is generic over the source-reference type, so
every backend reuses the same `$VAR` / `${VAR}` / `${VAR:-word}` /
`${VAR:+alt}` / `$` escaping logic while carrying its own provenance.
Definitions are inserted with a **precedence** (layer order) and an
**order** (sequence within a layer); `interpolate()` resolves a value and
records every reference with its source.

Compose builds one context from host env + interpolation files; GitLab and
CircleCI collect definitions first and interpolate in a **second pass**,
because their variables may reference each other in any order.

## The four backends

Each backend produces a report of variables with `winner` + `candidates`
(disposition `Winner`/`Shadowed`) + `references`, then renders it through
scan/explain/audit/graph.

| Backend | Layers (low → high precedence) |
| --- | --- |
| Compose | env_file entries < service `environment:` (canonical Docker override checked via `docker compose config --format json`) |
| GitHub Actions | workflow env < job env < step env < step env file < workflow inputs; `GITHUB_ENV` writes flagged as runtime-only |
| GitLab CI | included files (local, max depth 5) < file globals < job vars; `include: remote/template` reported external; v2 `$[[ inputs.X ]]` refs tracked |
| CircleCI | executor environment < job environment/env list; `<< parameters.X >>` tracked to the declaration; contexts/pipeline vars external |

## yamlx — the tolerant YAML loader

`serde_yaml::from_str` rejects three things real CI files use:

1. **Multiple documents** (`---`): GitLab merges them in order, later keys
   winning (glab's `spec:` + pipeline file).
2. **Several `<<` merge keys in one mapping**: YAML 1.1 allows it (fdroid
   uses two per job). A custom deserializer folds duplicates into one
   sequence-valued merge; resolution follows YAML 1.1 semantics (explicit
   keys > earlier merges > later merges).
3. **Unknown custom tags** (`!reference`): preserved as their value.

All four backends parse through `yamlx::parse_merged`.

## Audit and rules

`audit` layers on top of analysis:

- **Security** (by value shape, not just name): plaintext secrets,
  placeholder values, secret-manager references (Vault/AWS/templating),
  credentials in URLs, PEM private keys, known secret formats (AWS `AKIA…`,
  GitHub `ghp_…`, Stripe, Slack, OpenAI).
- **Dead code**: shadowed env lines, unused interpolation variables
  (sensitive ones still get the security checks).
- **Correctness**: undefined interpolation variables, empty values
  (`KEY:` resolving to an empty string), missing env files (info, matching
  Docker's notice semantics — short-syntax env files are optional, only
  long-syntax `required: true` errors).
- **Rules** (`envorigin.toml`): required/prefix/forbidden/patterns/
  allowed/max_length, plus `unknown-rule-variable` — rule keys matching no
  variable are reported (typo guard), with `required`-named keys exempt.

`--fail-on <level>` turns the audit into a CI gate; `--format github`
emits workflow commands so findings annotate PR lines; `--ignore <code>`
exempts codes for onboarding legacy projects.

## Diff

`diff` compares dotenv files (drift = same key, different values; one-sided
keys per file) or two Compose projects' final resolved environments.
Sensitive values are fingerprinted unless `--show-values`. `--fail-on-drift`
exits 1 when any variable drifts — the CI drift gate.

## Graph

Mermaid provenance graphs: solid edges for winners, dashed for shadowed,
dashed-derived for dependencies. Syntax is guarded by
`scripts/validate-graphs.sh` against the official mermaid parser
(mermaid.ink's public API is defunct).

## LSP

`envorigin lsp` speaks the Language Server Protocol over stdio (tower-lsp):
hover (winner + value), go-to-definition (winner source line), and live
diagnostics after every edit, including unsaved buffers. Files route by
name: `compose.y{a,}ml`, `.github/workflows/**/*.y{a,}ml`, `.gitlab-ci.yml`,
`.circleci/config.yml`, `.env`. Any LSP-capable editor can attach.

## Release pipeline

`scripts/release.sh <version>`: clean-tree check → tests/clippy/fmt →
bump Cargo.toml + lockfile → tag → GitHub
release → `cargo publish` → Homebrew formula update (url + sha256) in the
FIERsity/homebrew-envorigin tap. `release.yml` fires on the release event
and attaches prebuilt binaries for 5 platforms (linux x86_64/arm64, macOS
arm64/x86_64, Windows x86_64) — consumed by the
[FIERsity/envorigin-action](https://github.com/FIERsity/envorigin-action)
wrapper, which audits repositories in one step.

crates.io rate-limits rapid versioning (about 10 versions per 24h); plan
releases in batches.

## Validation strategy

- **Unit + integration tests** (136): analyzers, yamlx, rules, graph, CLI
  end-to-end over fixtures, LSP sessions over stdio.
- **`scripts/validate-real-repos.sh`**: clones outline/cargo/uv/glab/fdroid/
  influxdb shallow and asserts invariants — no panics on 51 workflow files,
  no `empty-value` noise, `--debug` traces resolve, GitLab multi-doc and
  double-merge configs scan, CircleCI 1000-line config audits, known-good
  findings stay stable. Always rebuilds the release binary (a stale
  `target/release` silently tests ancient code).
- **`scripts/validate-graphs.sh`**: mermaid syntax via the official parser.
- **CodeQL** (`.github/workflows/codeql.yml`): Rust SAST on every PR, push
  to main, and weekly; combined with `cargo audit` in the release gate,
  the dependency tree and the source are both scanned.
- Real-repo sweeps have repeatedly found genuine bugs (multi-doc YAML,
  merge keys, missing env files, `release:` job names, v2 input syntax) —
  run them before release.
