export const window = {
  showInformationMessage: vi.fn(),
  showWarningMessage: vi.fn(),
  createOutputChannel: vi.fn(() => ({
    appendLine: vi.fn(),
    show: vi.fn(),
    clear: vi.fn(),
  })),
};

export const commands = {
  registerCommand: vi.fn(),
};

export const workspace = {
  getConfiguration: vi.fn(() => ({
    get: vi.fn(),
    inspect: vi.fn(),
  })),
  onDidChangeConfiguration: vi.fn(() => ({
    dispose: vi.fn(),
  })),
  // Mirrors the real API: returns the workspace folder containing the given URI.
  getWorkspaceFolder: vi.fn((uri: { fsPath: string }) => {
    const folders = (workspace as { workspaceFolders?: { uri: { fsPath: string } }[] }).workspaceFolders;
    return folders?.find(
      (folder) => uri.fsPath === folder.uri.fsPath || uri.fsPath.startsWith(`${folder.uri.fsPath}/`),
    );
  }),
};

export const Uri = {
  file: vi.fn((fsPath: string) => ({ fsPath })),
};

export const debug = {
  registerDebugConfigurationProvider: vi.fn(() => ({ dispose: vi.fn() })),
};

export const DebugConfigurationProviderTriggerKind = {
  Initial: 1,
  Dynamic: 2,
};

// Modules under test reach for vscode both as `import * as vscode` and as a default import;
// the real extension host module satisfies both.
export default { window, commands, workspace, debug, DebugConfigurationProviderTriggerKind, Uri };
