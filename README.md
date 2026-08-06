# EnvOrigin

[![CI](https://github.com/FIERsity/envorigin/actions/workflows/ci.yml/badge.svg)](https://github.com/FIERsity/envorigin/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/envorigin)](https://crates.io/crates/envorigin)
[![Downloads](https://img.shields.io/crates/d/envorigin)](https://crates.io/crates/envorigin)
[![Homebrew](https://img.shields.io/badge/dynamic/json?url=https://formulae.brew.sh/api/formula/envorigin.json&query=$.versions.stable&label=homebrew)](https://github.com/FIERsity/homebrew-envorigin)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

Explain where environment variables actually come from — in Docker Compose
projects **and** GitHub Actions workflows.

> “Why is `DATABASE_URL` suddenly `localhost` in staging? It was fine yesterday.”
> “Which `SHARED` did that step actually see — the workflow env, the job env,
> the step env, or the env file?”

A variable in a Compose service can be defined in five places at once — your
shell, `.env`, `--env-file`, `services.*.env_file`, and `services.*.environment`
— and Compose resolves each one differently depending on how the others are
written. A workflow variable can be defined in five more: workflow, job, and
step `env:` blocks, `env: file:` references, and `GITHUB_ENV` writes at
runtime. When the value changes, figuring out *which* layer won (and which
layers you just wasted an hour editing) is manual archaeology.

EnvOrigin reads your project exactly the way the platform does and answers the
question per variable, with the full provenance chain and line numbers:

```text
$ envorigin explain P -s web --show-values

P for service web
state: present
value: "compose_explicit"
winner: service environment (compose.yaml:9)
candidates:
  - [shadowed] service env_file (env/web.env:1) = "from_web_env"
  - [shadowed] service env_file (env/overrides.env:1) = "from_overrides_env"
  - [winner] service environment (compose.yaml:9) = "compose_explicit"
```

Values are redacted by default — you get a short SHA-256 fingerprint instead of
the secret. Pass `--show-values` only when you mean it.

## Why

Docker's own precedence rules are subtle and their failure modes are loud:

| Layer (low → high) | Writes a value | Notes |
| --- | --- | --- |
| Compose file defaults | rarely | only via interpolation |
| Shell / process environment | when present | wins over `.env` |
| `.env` / `--env-file` | interpolation only | **never injected into the container directly** |
| `services.*.env_file` | injected | later files override earlier ones |
| `services.*.environment` | injected | explicit values override everything |
| `docker compose config` | ground truth | what Compose actually computed |

Two consequences that bite teams every week:

1. **`.env` is not an injection layer.** Values there only reach the container
   through `${VAR}` interpolation or an unset `environment: - VAR` entry.
   Editing `.env` and seeing no effect is the most common false fix.
2. **The winner is invisible.** If `environment:` overrides a value from
   `env_file`, Compose gives you no hint that the env_file line is dead code.
   EnvOrigin marks it `shadowed` so you can delete it with confidence.

## Install

```sh
cargo install envorigin            # crates.io
brew install FIERsity/envorigin/envorigin  # Homebrew
# or from source
cargo build --release
```

Every [GitHub Release](https://github.com/FIERsity/envorigin/releases) also
carries prebuilt binaries — no Rust toolchain or Homebrew needed:

| Platform | Asset |
| --- | --- |
| Linux x86_64 | `envorigin-<version>-x86_64-unknown-linux-gnu` |
| Linux arm64 | `envorigin-<version>-aarch64-unknown-linux-gnu` |
| macOS arm64 | `envorigin-<version>-aarch64-apple-darwin` |
| macOS x86_64 | `envorigin-<version>-x86_64-apple-darwin` |
| Windows x86_64 | `envorigin-<version>-x86_64-pc-windows-msvc.exe` |

```sh
# e.g. CI cache or a Docker layer
curl -sL https://github.com/FIERsity/envorigin/releases/download/v1.11.0/envorigin-v1.11.0-x86_64-unknown-linux-gnu -o envorigin
chmod +x envorigin
```

Requires Rust 1.86+.

## Usage

```text
EnvOrigin 0.3.0
Explain where environment variables come from

Usage: envorigin <COMMAND>

Commands:
  scan     List the variables configured for each Compose service
  explain  Explain the winning and shadowed sources for one variable
  audit    Audit a project for env health issues (sensitive values, dead code)
  diff     Compare environment drift across dotenv files
  graph    Render a mermaid provenance graph of the environment
  gitlab   Analyze a GitLab CI configuration's variables
  circleci Analyze a CircleCI configuration's environment variables
  actions  Analyze a GitHub Actions workflow's environment variables
  lsp      Start the LSP server (for editor integration)
  completions  Generate a shell completion script
  init      Generate an envorigin.toml rules template
  dotenv    Audit standalone dotenv files without a Compose context
  help     Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

Common flags for `scan`/`explain`:

| Flag | Meaning |
| --- | --- |
| `-f, --file <PATH>` | Compose file to analyze (default `compose.yaml`) |
| `--env-file <PATH>` | Compose interpolation file; repeatable, later files win |
| `--project-directory <PATH>` | override the Compose project directory |
| `--host-env-file <PATH>` | overlay the shell from a dotenv file (reproducible diagnosis) |
| `--no-docker-check` | skip comparing with `docker compose config` |
| `--show-values` | reveal plaintext values (default: redacted) |
| `--format json` | machine-readable output |

### `scan`

```text
$ envorigin scan

EnvOrigin 0.3.0
compose: /app/compose.yaml
docker verification: verified
interpolation files: /app/.env

service web (2 present, 4 tracked)
  D                        absent  —                        ← service environment (compose.yaml:8)
  FROM_ENV                 set     <redacted sha256:b3aa92d6> ← service environment (compose.yaml:6)
  MODE                     set     <redacted sha256:ab8e18ef> ← service environment (compose.yaml:5)
```

`docker verification` reports whether your understanding matched
`docker compose config --format json`:

- `verified` — EnvOrigin's model agrees with Docker (run it on a real project
  and it should normally say this; if it doesn't, that's a bug — please report)
- `unavailable` — Docker CLI not installed; analysis is unverified
- `failed` — Docker ran but disagreed, with a `docker-divergence` diagnostic
  on each affected variable
- `skipped` — `--no-docker-check` was passed

### `explain`

```text
$ envorigin explain T -s web --show-values

T for service web
state: present
value: "from_project_env"
winner: service env_file (env/web.env:4)
derived from:
  - default .env (.env:2)
```

`T=${S}` in `env/web.env` interpolated `S` from `.env` — the `derived from`
block is EnvOrigin's answer to "but *why* is it that value?" for interpolated
variables. `explain --debug` prints the full resolution trace: every
definition of the variable in the interpolation context with its
precedence and order.

### `actions`

GitHub Actions resolves a variable from up to five declaration layers per
scope, and one runtime layer you can't see statically:

| Layer (low → high) | Writes a value | Notes |
| --- | --- | --- |
| Workflow-level `env:` | every job and step | |
| Job-level `env:` | every step in the job | overrides workflow env |
| Job-level `env: file:` | every step in the job | overrides job env |
| Step-level `env:` | that step | overrides job layers |
| Step-level `env: file:` | that step | overrides step env |
| `GITHUB_ENV` writes | subsequent steps | **runtime only — flagged, not resolved** |

```text
$ envorigin actions explain SHARED -j build -s Configure --show-values

SHARED in workflow .github/workflows/ci.yml
state: set
value: "step_value"
winner: step env (.github/workflows/ci.yml:19)
candidates:
  - [shadowed] workflow env (.github/workflows/ci.yml:4) = "workflow_value"
  - [shadowed] job env (.github/workflows/ci.yml:11) = "job_value"
  - [winner] step env (.github/workflows/ci.yml:19) = "step_value"
```

`--debug` adds a layer-by-layer resolution trace (which definition wins at
each precedence level):

```text
$ envorigin actions explain SHARED -j build -s Configure --debug

resolution trace for SHARED (workflow env < job env < step env < env file < inputs; later wins):
  #1   shadowed  workflow env (.github/workflows/ci.yml:4)   "workflow_value"
  #2   shadowed  job env (.github/workflows/ci.yml:11)       "job_value"
  #3   winner    step env (.github/workflows/ci.yml:19)      "step_value"
  3 definition(s)
```

`actions` also tracks `${{ ... }}` expressions — the reference list answers
"what did this value read from?" for `${{ env.X }}` (resolved to its source),
`${{ secrets.X }}` / `${{ vars.X }}` / `${{ github.* }}` (marked `external
context`, since their values live in repository/org settings), and
`${{ matrix.X }}` (marked external — a strategy variable). `GITHUB_ENV` writes
like `echo "RESULT=ok" >> "$GITHUB_ENV"` produce an Info diagnostic: the
variable exists at runtime but cannot be resolved statically.

Flags: `-f/--file` (default `.github/workflows/ci.yml`), `-j/--job`,
`-s/--step` (name, id, or index), `--project-directory`,
`--show-values`, `--format json`.

### `audit` / `actions audit` / `gitlab audit` / `circleci audit`

A checkable health report for a whole project. It aggregates every
diagnostic the analyzer produces, then adds checks a diagnostic model cannot
express:

| Check | Severity | What it reports |
| --- | --- | --- |
| `sensitive-value` | error | a variable whose name matches `TOKEN`/`SECRET`/`PASSWORD`/`KEY`/… is set to a concrete value |
| `undefined-interpolation-variable` | warning | `$VAR` / `${VAR}` references a variable that resolves to empty |
| `shadowed-env-line` | warning | a definition loses to a higher-precedence layer with a different value — dead code |
| `secret-manager-reference` | info | a sensitive variable references an external secret (Vault, AWS Secrets Manager/SSM, templating) — its value cannot be resolved statically |
| `empty-value` | info | a variable resolves to an empty string — usually an accidental stub |
| `credential-in-url` | error | any variable embeds credentials in a URL (`scheme://user:pass@host`) — the most common leak, usually under names like `DATABASE_URL` |
| `private-key-in-value` | error | any variable embeds a PEM private key (`-----BEGIN ... PRIVATE KEY-----`) |
| `known-secret-format` | error | any variable contains a known credential shape (AWS `AKIA…`, GitHub `ghp_…`, Stripe `sk_live_…`, Slack, OpenAI) — detected by value, not name |
| `unused-interpolation-variable` | info | an interpolation-file variable no service consumes (sensitive ones also get the placeholder/value checks — a plaintext secret is flagged even when unused) |
| `dotenv-trailing-content`, docker checks, … | as diagnosed | existing analyzer diagnostics |

### Secret managers (Vault, AWS Secrets Manager/SSM)

EnvOrigin never resolves external secrets — it detects that a value *is* an
external reference and reports it as `secret-manager-reference` (info), so
pipelines don't treat a placeholder as a real credential:

| Value shape | Detected as |
| --- | --- |
| `vault:secret/data/db#password` | HashiCorp Vault |
| `arn:aws:secretsmanager:region:…:secret:name` | AWS Secrets Manager |
| `arn:aws:ssm:region:…:parameter/name` / `sm://…` | AWS SSM |
| `{{ secret … }}` / `{{ secrets.… }}` | secret templating |

The recommended pattern is to keep the reference in the config and store
the real value out-of-band:

```yaml
# docker-compose.yml — the reference is intentional, no secret here
services:
  web:
    environment:
      DB_PASSWORD: vault:secret/data/db#password
      AWS_CREDENTIALS: arn:aws:secretsmanager:us-east-1:123456789012:secret:prod/credentials
```

`envorigin audit` reports both as `info [secret-manager-reference]` (no
pipeline failure by default). Rotate any *concrete* value the audit flags
as `sensitive-value` into one of these references, then re-run
`envorigin audit --fail-on error` to confirm.

```text
$ envorigin audit

EnvOrigin 0.3.0 — audit
compose: /app/compose.yaml

2 error(s), 2 warning(s), 1 info

error [sensitive-value]: API_TOKEN is set to a concrete value; verify it is not a real credential (/app/compose.yaml:7)
warning [shadowed-env-line]: SHADOWED is shadowed by a higher-precedence layer and can be removed (/app/env/web.env:2)
info [unused-interpolation-variable]: ORPHAN in the interpolation file is not consumed by any service (/app/.env:2)
```

All audit commands support `--format json` (issues array) and
`--format github` (GitHub Actions workflow commands — problems appear
annotated on the offending file lines in pull requests). `--ignore <code>`
(repeatable) exempts issue codes — use it to onboard legacy projects and
remove ignores as problems get fixed.

`--fail-on <level>` (default `error`) turns the audit into a CI gate:
`--fail-on warning` exits 1 when any warning or error is found. Values are
never printed — the audit can run on repositories holding real credentials.

### Rules (`envorigin.toml`)

`envorigin init` writes a commented rules template (all six rule types)
into the current directory; it never overwrites an existing file.

Turn team conventions into enforced audit checks. Any audit command
(`audit`, `actions audit`, `gitlab audit`, `circleci audit`) loads
`./envorigin.toml` automatically, or a file via `--config`:

```toml
# envorigin.toml
required = ["DATABASE_URL", "LOG_LEVEL"]   # must resolve somewhere
prefix = "APP_"                            # every user variable must follow
forbidden = ["CI"]                         # must not be defined

[patterns]
DATABASE_URL = "^postgres(ql)?://"         # value format validation
LOG_LEVEL = "^(debug|info|warn|error)$"

[allowed]
DB_ENGINE = ["postgres", "mysql"]      # enum whitelist

[max_length]
API_KEY = 64                            # length cap
```

| Check | Severity | Trigger |
| --- | --- | --- |
| `required-variable-missing` | error | a required name resolves nowhere |
| `naming-prefix` | warning | a user-defined variable violates the prefix |
| `forbidden-variable` | error | a forbidden name is defined |
| `pattern-mismatch` | error | a variable's value fails its `[patterns]` regex |
| `disallowed-value` | error | a variable's value is not in its `[allowed]` set |
| `value-too-long` | error | a variable exceeds its `[max_length]` cap |
| `unknown-rule-variable` | warning | a `[patterns]`/`[allowed]`/`[max_length]` key matches no variable — usually a typo, and the rule would silently never fire |

The same checks apply to all four backends; `--fail-on` gates CI exactly as
for the built-in checks. Rule keys for `required` variables are exempt from
`unknown-rule-variable`: declaring a convention for a variable the project
must add is the intended setup.

EnvOrigin audits its own workflow on every push: the `Audit own workflow
(dogfood)` step in `.github/workflows/ci.yml` runs
`envorigin actions audit --fail-on warning` on the repository's CI file.

### `dotenv audit`

Audit standalone `.env` files with no Compose context — every entry gets
the full sensitive-value / placeholder / secret-manager / URL-credential /
private-key / known-format checks, plus `--fail-on`, `--format`, and
`envorigin.toml` rules (`--config`):

```text
$ envorigin dotenv audit .env

2 error(s), 1 warning(s), 0 info
warning [sensitive-placeholder]: DB_PASSWORD is set to a placeholder-looking value ... (.env:1)
error [known-secret-format]: AWS_KEY contains what looks like a AWS access key ... (.env:2)
error [credential-in-url]: URL embeds credentials in a URL ... (.env:3)
```

### `diff`

Compare dotenv files (below) **or** two Compose projects' final resolved
environments:

```text
$ envorigin diff --project-a ./local --project-b ./prod

drift (1):
  web.DB_HOST: ./local = "localhost" | ./prod = "db.prod.example.com"
only in ./local (1):
  web.ONLY_A = "a_value"
```

Both modes support `--format json` (machine-readable; sensitive values stay
redacted unless `--show-values`) and `--fail-on-drift`, which exits 1 when any
variable carries different values — a CI gate that catches environments
drifting apart:

```yaml
# ci/drift.yml — fail the pipeline when local and prod diverge
steps:
  - run: envorigin diff --project-a ./local --project-b ./prod --fail-on-drift --format json
```

### `diff` (dotenv files)

Compare dotenv files for environment drift — the same variable with different
values across local, CI, and staging:

```text
$ envorigin diff .env ci.env

EnvOrigin 0.3.0 — diff
.env vs ci.env

drift (2):
  API_TOKEN: .env = <redacted sha256:61be8cd3> | ci.env = <redacted sha256:9b723f99>
  DB_HOST: .env = "localhost" | ci.env = "db.prod.example.com"

only in .env (1):
  ONLY_LOCAL = "local_value"
```

Sensitive-looking variables are redacted unless `--show-values` is passed.

### `graph`

A mermaid provenance graph — pipe it into [mermaid-cli](https://github.com/mermaid-js/mermaid-cli),
mermaid.ink, or any mermaid renderer:

```text
$ envorigin graph | mmdc -o provenance.svg
```

Solid edges point at the winning source, dashed edges at shadowed candidates
(dashed with a `derived` label at interpolation dependencies). `actions graph`
renders the workflow's job/step/variable/source layout; `gitlab graph` and
`circleci graph` render the same provenance for their backends. Output has
been validated against the mermaid renderer for all four.

### `gitlab`

Layered resolution for `.gitlab-ci.yml` `variables:`: included files
(`include: local`) < file-global < job-level. `$VAR` / `${VAR}` interpolation
is tracked per reference, and — unlike Compose — definitions may reference
each other in any order (GitLab semantics), so values are resolved in a
second pass over all definitions. `include: remote` / `include: template`
are reported as external and not fetched. Statically-known predefined
variables (`CI_*`, `GITLAB_*`, `RUNNER_*`) are labelled as GitLab predefined.

```text
$ envorigin gitlab explain BUILD_ID -j build --show-values

BUILD_ID in .gitlab-ci.yml
state: set
value: "-demo"
winner: job variables (.gitlab-ci.yml:20)
references:
  - $CI_PIPELINE_ID → GitLab predefined (GitLab predefined)
  - $APP_NAME → global variables (.gitlab-ci.yml:7)
```


### `circleci`

Layered resolution for `.circleci/config.yml`: executor `environment:` < job
`environment:` / `env:` list. `<< parameters.X >>` references resolve to the
job's `parameters:` declaration (default shown); `<< pipeline.X >>` and
`context:` are organization/API state and are reported as external.
`CIRCLE_*` predefined variables are labelled; values interpolate shell-style.

```text
$ envorigin circleci explain TARGET -j build --show-values

TARGET in .circleci/config.yml
state: set
value: "<< parameters.target >>"
winner: job environment (.circleci/config.yml:21)
references:
  - parameters.target → job parameter (.circleci/config.yml:15)
```


### `lsp` — editor integration

`envorigin lsp` speaks the Language Server Protocol over stdio and powers the
[VS Code extension](vscode/):

- **Hover** a variable definition line to see its winning source and value
  (redacted by default).
- **Go to definition** jumps from a variable to the file and line that won.
- **Live diagnostics** mirror the audit findings — undefined interpolation
  references, shadowed dead-code lines, sensitive values — as you edit.

Documents are routed to the matching backend by file name (`compose.y*ml`,
`.github/workflows/*.yml`, `.gitlab-ci.yml`, `.circleci/config.yml`, `.env`
— dotenv files get the live security diagnostics).
Unsaved buffer edits are analyzed in memory — hover and diagnostics track
the buffer, while relative references still resolve against the real file
location. The extension lives in [`vscode/`](vscode/) (TS client, `npm run compile` + `npx @vscode/vsce package` to build, `binaryPath` setting to point at the CLI). Install the built VSIX with `code --install-extension envorigin-vscode-<version>.vsix`.

## Design

- **`src/compose.rs`** — Compose model (env_file specs, environment forms) and
  per-variable candidate resolution in Compose's own order.
- **`src/interpolation.rs`** — a Compose-flavored interpolator: `$VAR`,
  `${VAR}`, `${VAR:-word}`, `${VAR:?msg}`, `${VAR:+alt}`, `$$` escaping, with
  per-reference source tracking.
- **`src/dotenv.rs`** — dotenv parsing: quoting, `export` prefixes, comments,
  unset entries, trailing-content warnings.
- **`src/docker.rs`** — spawns `docker compose config --format json` and
  reconciles Docker's canonical model against the local model.
- **`src/actions.rs`** — workflow model: layered env resolution (workflow →
  job → job file → step → step file), `${{ }}` reference tracking, built-in
  default variables, and `GITHUB_ENV` runtime detection.
- **`src/output.rs`** — human and JSON renderers, default redaction with
  SHA-256 fingerprints.

Semantics notes:

- `COMPOSE_ENV_FILES` is expanded (path-separator split, relative to the
  project directory, replacing the default `.env` lookup); missing entries
  produce a `compose-env-files-missing` warning and are skipped. Explicit
  `--env-file` arguments take precedence over it, mirroring Docker Compose.
- `COMPOSE_FILE` is not followed; it produces a `compose-file-redirect`
  warning telling you to pass `--file` explicitly.
- `actions` is static analysis: `GITHUB_ENV` writes are flagged as
  runtime-only, and GitHub does not document the precedence of a `file:` key
  mixed with regular keys in one `env:` block — EnvOrigin applies the file
  layer after the map (same order as Compose `env_file`).

## Performance

A synthetic project with 800 variables across 4 services plus 4 env files
(821-line compose) scans in ~0.3 s and audits in ~2.4 s (debug build;
release builds are several times faster) — linear in input size, no
caching needed.

## Testing

```sh
cargo test
```

112 tests: unit tests for the dotenv parser, the interpolator, Compose
normalization, the Docker canonical extraction, the Actions layer
resolution, and the audit checks; plus end-to-end CLI tests against
`tests/fixtures/{basic,precedence,raw,env-files,actions,actions-inputs,audit}`
that assert redaction, derivation tracking, the full shadowing chain,
`COMPOSE_ENV_FILES` expansion, `format: raw` semantics,
expression-reference resolution, and audit exit codes. CI runs `fmt` +
`clippy -D warnings` + tests on Ubuntu, and audits its own workflow
(dogfood).

## Roadmap

**Phase 1 — doctor mode (in progress)**
- [x] `audit` with sensitive-value, shadowed-dead-code, unused-variable checks
- [x] `--fail-on` CI gate + dogfooding EnvOrigin's own workflow
- [x] `envorigin diff` — compare dotenv files for environment drift
      (sensitive values redacted by default)
- [x] real-repository validation: audited all 39 examples in
      docker/awesome-compose (every file parses and resolves; the findings fixed the
      placeholder and unused-variable checks). The GitLab and CircleCI
      backends were validated against real production configs: GitLab's own
      Auto-DevOps template, and leiningen's JSON-format CircleCI config.

**Phase 2 — visualization & IDE (in progress)**
- [x] `envorigin graph` / `envorigin actions graph` — mermaid provenance
      graph (solid = winner, dashed = shadowed, dashed-derived = dependency);
      output validated against the mermaid renderer
- [x] LSP server + VS Code extension (hover, go-to-definition, live diagnostics)
- [x] real CI config validation: rust-lang/cargo's 372-line workflow and
      astral-sh/uv's benchmark workflow (with `workflow_dispatch` inputs)
      parse, scan, and audit cleanly; unnamed steps display 1-based numbers
- [x] GitLab CI variables support (include < global < job; `$VAR`
      reference tracking; predefined variables)
- [x] CircleCI env support (executor < job, parameters, contexts external)

**Phase 3 — ecosystem**
- [x] crates.io release (`cargo install envorigin`) + Homebrew tap
- [x] rules engine: `envorigin.toml` for team conventions (required vars,
      name prefixes, value validation)
- [x] prebuilt release binaries for linux/macOS/Windows (no toolchain needed)
- [x] GitHub Action wrapper (`FIERsity/envorigin-action`, dogfooded in this
      repo's own CI)
- [x] integration notes for secret managers (Vault, AWS SSM)

### Shell completions

```sh
envorigin completions bash  > ~/.bash_completion.d/envorigin
envorigin completions zsh   > ~/.zfunc/_envorigin
envorigin completions fish  > ~/.config/fish/completions/envorigin.fish
```

## CI integration

`audit` is built to gate pipelines: `--format json` for parsing,
`--fail-on <level>` for the exit code. Wire it into any of the three CI
platforms EnvOrigin understands:

**GitHub Actions**

The one-step wrapper (no Rust toolchain; downloads the prebuilt binary,
auto-detects the config, annotates findings on the PR):

```yaml
- uses: FIERsity/envorigin-action@v1
  with:
    fail-on: warning
```

Or the CLI directly:

```yaml
- name: Install envorigin
  run: cargo install envorigin
- name: Audit environment config
  run: envorigin audit --fail-on warning
```

**GitLab CI**

```yaml
audit:
  script:
    - cargo install envorigin
    - envorigin gitlab audit --fail-on warning
  allow_failure: false
```

**CircleCI**

```yaml
jobs:
  audit:
    docker:
      - image: cimg/rust:1.86
    steps:
      - checkout
      - run: cargo install envorigin
      - run: envorigin circleci audit --fail-on warning
```

Each audit command targets its own platform's config file by default
(`compose.yaml`, `.github/workflows/ci.yml`, `.gitlab-ci.yml`,
`.circleci/config.yml`) and applies `envorigin.toml` rules automatically.

## Real-world example

Regression validation against live open-source repositories is one
command — `scripts/validate-real-repos.sh` clones outline, cargo, uv, and
two GitLab-hosted repos (glab, fdroid) shallow, sweeps 51 workflow files
for panics and noise, checks `explain --debug` traces on multi-layer
workflows, validates the GitLab backend against real multi-document and
double-merge-key configs, and pins the known-good findings (outline's CI
embeds test credentials and must keep flagging them).

Running the released binary against [outline](https://github.com/outline/outline)'s
`docker-compose.yml` (production knowledge-base app) with its `.env`:

```text
$ envorigin audit -f docker-compose.yml --project-directory .

0 error(s), 1 warning(s), 87 info

warning [sensitive-placeholder]: POSTGRES_PASSWORD is set to a placeholder-looking value; replace it before deploying (docker-compose.yml:13)
info [unused-interpolation-variable]: NODE_ENV in the interpolation file is not consumed by any service (.env:1)
info [unused-interpolation-variable]: URL in the interpolation file is not consumed by any service (.env:22)
info [unused-interpolation-variable]: PORT in the interpolation file is not consumed by any service (.env:26)
info [unused-interpolation-variable]: COLLABORATION_URL in the interpolation file is not consumed by any service (.env:30)
```

The audit reads the same way a maintainer would: the 312-line `.env` is
almost entirely runtime configuration for the application, and only a
handful of variables ever reach the Compose services — a real, immediately
actionable answer to "why doesn't my .env change anything?".

```text
$ envorigin explain POSTGRES_PASSWORD -s postgres

POSTGRES_PASSWORD for service postgres
state: present
value: <redacted sha256:d74ff0ee>
winner: service environment (docker-compose.yml:13)
candidates:
  - [winner] service environment (docker-compose.yml:13) = <redacted sha256:d74ff0ee>
```

## Release

`./scripts/release.sh <version>` tags, creates the GitHub release, publishes to
crates.io, and updates the Homebrew formula in one run.

## License

MIT
