import * as vscode from "vscode";

import { prepareDerivedDataPath, workspaceFoldersContaining } from "../build/utils";
import { getWorkspaceConfig } from "../common/config";
import { commonLogger } from "../common/logger";

export class TuistGenWatcher implements vscode.Disposable {
  private watchers: vscode.FileSystemWatcher[] = [];
  // One pending generate per folder: a change in one project must not cancel another's.
  private throttles = new Map<string, NodeJS.Timeout>();
  private derivedDataPath: string | null = null;

  async start(): Promise<void> {
    this.derivedDataPath = prepareDerivedDataPath();
    // Is config enabled?
    // TODO: add config to enable/disable watcher
    const isEnabled = getWorkspaceConfig("tuist.autogenerate");
    if (!isEnabled) {
      return;
    }

    // Every folder holding Tuist manifests gets its own watcher, scoped to that folder, so the
    // generate runs in the directory whose manifests triggered it.
    const folders = await workspaceFoldersContaining("Project.swift", "Workspace.swift");
    if (folders.length === 0) {
      commonLogger.log("Project.swift or Workspace.swift not found, skipping tuist watcher");
      return;
    }

    for (const folder of folders) {
      const root = folder.uri.fsPath;
      const swiftWatcher = vscode.workspace.createFileSystemWatcher(
        new vscode.RelativePattern(folder, "**/*.swift"),
        false, // ignoreCreateEvents
        true, // ignoreChangeEvents
        false, // ignoreDeleteEvents
      );
      swiftWatcher.onDidCreate((e) => this.handleChange(root, e));
      swiftWatcher.onDidDelete((e) => this.handleChange(root, e));
      this.watchers.push(swiftWatcher);
    }

    commonLogger.log("tuist watcher started", {
      roots: folders.map((folder) => folder.uri.fsPath),
    });
  }

  handleChange(root: string, e: vscode.Uri) {
    commonLogger.log("tuist watcher detected changes", {
      root: root,
      file: e.fsPath,
    });

    // Skip files created in derived data path
    if (this.derivedDataPath && e.fsPath.startsWith(this.derivedDataPath)) {
      return;
    }

    const pending = this.throttles.get(root);
    if (pending) {
      clearTimeout(pending);
    }

    this.throttles.set(
      root,
      setTimeout(() => {
        this.throttles.delete(root);
        // The command reports its own failures to the user, so this only records that it ran.
        Promise.resolve(vscode.commands.executeCommand("sweetpad.tuist.generate", root))
          .then(() => {
            commonLogger.log("tuist generate finished", {
              root: root,
            });
          })
          .catch((error) => {
            commonLogger.error("Failed to generate tuist project", {
              root: root,
              error: error,
            });
          });
      }, 1000 /* 1s */),
    );
  }

  dispose(): void {
    for (const timeout of this.throttles.values()) {
      clearTimeout(timeout);
    }
    this.throttles.clear();
    for (const watcher of this.watchers) {
      watcher.dispose();
    }
  }
}
