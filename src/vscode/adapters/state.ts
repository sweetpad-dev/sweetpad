import type * as vscode from "vscode";

import type { WorkspaceState, WorkspaceStateKey, WorkspaceTypes } from "../../core/state/types";

const PREFIX = "sweetpad.";

/**
 * VS Code-backed WorkspaceState. Stores values in the extension's
 * `workspaceState` Memento under the `sweetpad.` prefix.
 */
export class VsCodeWorkspaceState implements WorkspaceState {
  constructor(private readonly vscodeContext: vscode.ExtensionContext) {}

  get<K extends WorkspaceStateKey>(key: K): WorkspaceTypes[K] | undefined {
    return this.vscodeContext.workspaceState.get(`${PREFIX}${key}`);
  }

  update<K extends WorkspaceStateKey>(key: K, value: WorkspaceTypes[K] | undefined): void {
    this.vscodeContext.workspaceState.update(`${PREFIX}${key}`, value);
  }

  /**
   * Remove all sweetpad.* keys from workspace state.
   */
  reset(): void {
    for (const key of this.vscodeContext.workspaceState.keys()) {
      if (key?.startsWith(PREFIX)) {
        this.vscodeContext.workspaceState.update(key, undefined);
      }
    }
  }
}
