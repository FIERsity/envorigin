# EnvOrigin for VS Code

Hover, go-to-definition, and live diagnostics for environment variables in
Docker Compose, GitHub Actions, GitLab CI, and CircleCI configuration files,
backed by the `envorigin` LSP server.

## Install

1. Build and install the CLI: `cargo install --path ..` (or add the binary to
   your `PATH`).
2. In VS Code, set `envorigin.binaryPath` to the binary location if it is not
   on your `PATH`.

## Build the extension (VSIX)

```sh
npm install
npm run compile
npx @vscode/vsce package
```

Then `code --install-extension envorigin-vscode-0.3.0.vsix`.

## Features

- **Hover** a variable definition line to see its winner source and value
  (redacted by default).
- **Go to definition** jumps from a variable to the file and line that won
  the resolution.
- **Live diagnostics** mark undefined interpolation references, shadowed
  dead-code lines, and sensitive values as you edit.
