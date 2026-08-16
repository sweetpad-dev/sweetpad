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
  onDidChangeWorkspaceFolders: vi.fn(() => ({
    dispose: vi.fn(),
  })),
  // Mirrors the real API, which looks the URI up in a prefix tree over the folder URIs: when
  // folders nest ("/repo" and "/repo/ios"), the innermost containing folder wins regardless of the
  // order they were added, and path comparison ignores case as the macOS file system provider does.
  getWorkspaceFolder: vi.fn((uri: { fsPath: string }) => {
    const folders = (workspace as { workspaceFolders?: { uri: { fsPath: string } }[] }).workspaceFolders;
    const target = uri.fsPath.toLowerCase();
    let match: { uri: { fsPath: string } } | undefined;
    for (const folder of folders ?? []) {
      const root = folder.uri.fsPath.toLowerCase();
      if (target !== root && !target.startsWith(`${root}/`)) {
        continue;
      }
      if (match === undefined || folder.uri.fsPath.length > match.uri.fsPath.length) {
        match = folder;
      }
    }
    return match;
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
