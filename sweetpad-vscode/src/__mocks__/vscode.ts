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
export default { window, commands, workspace, debug, DebugConfigurationProviderTriggerKind };
