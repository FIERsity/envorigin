import * as vscode from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

export function activate(context: vscode.ExtensionContext): void {
  const configured = vscode.workspace
    .getConfiguration('envorigin')
    .get<string>('binaryPath', 'envorigin');
  const binary = configured || 'envorigin';

  const serverOptions: ServerOptions = {
    command: binary,
    args: ['lsp'],
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { pattern: '**/compose.y{a,}ml' },
      { pattern: '**/docker-compose.y{a,}ml' },
      { pattern: '**/.github/workflows/**/*.yml' },
      { pattern: '**/.gitlab-ci.yml' },
      { pattern: '**/.circleci/config.yml' },
    ],
  };

  client = new LanguageClient('envorigin', 'EnvOrigin', serverOptions, clientOptions);
  client.start();
  context.subscriptions.push(client);
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
