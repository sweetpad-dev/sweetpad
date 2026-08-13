import * as vscode from "vscode";

import { getWorkspacePath, isFileExistsInAnyWorkspaceFolder, prepareDerivedDataPath } from "../build/utils";
import { getWorkspaceConfig } from "../common/config";
import { commonLogger } from "../common/logger";

export class TuistGenWatcher implements vscode.Disposable {
  private watchers: vscode.FileSystemWatcher[] = [];
  private throttle: NodeJS.Timeout | null = null;
  private derivedDataPath: string | null = null;
  private workspacePath = "";

  async start(): Promise<void> {
    this.derivedDataPath = prepareDerivedDataPath();
    this.workspacePath = getWorkspacePath();
    // Is config enabled?
    // TODO: add config to enable/disable watcher
    const isEnabled = getWorkspaceConfig("tuist.autogenerate");
    if (!isEnabled) {
      return;
    }

    // We don't even need to start the watcher if there is no tuist files in any workspace folder
    const isTuistFileExists =
      (await isFileExistsInAnyWorkspaceFolder("Project.swift")) ||
      (await isFileExistsInAnyWorkspaceFolder("Workspace.swift"));
    if (!isTuistFileExists) {
      commonLogger.log("Project.swift or Workspace.swift not found, skipping tuist watcher", {
        workspacePath: getWorkspacePath(),
      });
      return;
    }

    const swiftWatcher = vscode.workspace.createFileSystemWatcher(
      "**/*.swift",
      false, // ignoreCreateEvents
      true, // ignoreChangeEvents
      false, // ignoreDeleteEvents
    );
    swiftWatcher.onDidCreate((e) => this.handleChange(e));
    swiftWatcher.onDidDelete((e) => this.handleChange(e));
    this.watchers.push(swiftWatcher);

    commonLogger.log("tuist watcher started", {
      workspacePath: getWorkspacePath(),
    });
  }

  handleChange(e: vscode.Uri) {
    commonLogger.log("tuist watcher detected changes", {
      workspacePath: this.workspacePath,
      file: e.fsPath,
    });
    if (this.throttle) {
      clearTimeout(this.throttle);
    }

    // Skip files created in derived data path
    if (this.derivedDataPath && e.fsPath.startsWith(this.derivedDataPath)) {
      return;
    }

    this.throttle = setTimeout(() => {
      this.throttle = null;
      Promise.resolve(vscode.commands.executeCommand("sweetpad.tuist.generate"))
        .then(() => {
          commonLogger.log("tuist project was successfully generated", {
            workspacePath: this.workspacePath,
          });
        })
        .catch((error) => {
          commonLogger.error("Failed to generate tuist project", {
            workspacePath: this.workspacePath,
            error: error,
          });
        });
    }, 1000 /* 1s */);
  }

  dispose(): void {
    for (const watcher of this.watchers) {
      watcher.dispose();
    }
  }
}
