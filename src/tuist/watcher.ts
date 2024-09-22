import path from "node:path";
import * as vscode from "vscode";
import { Disposable } from "vscode";
import { getWorkspacePath, prepareDerivedDataPath } from "../build/utils";
import type { ExtensionContext } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { isFileExists } from "../common/files";
import { commonLogger } from "../common/logger";
import { tuistGenerateCommand } from "./command";

class TuistGenWatcher {
  private watchers: vscode.FileSystemWatcher[] = [];
  private throttle: NodeJS.Timeout | null = null;
  private derivedDataPath: string | null;
  private workspacePath: string;

  constructor(private extension: ExtensionContext) {
    this.derivedDataPath = prepareDerivedDataPath();
    this.workspacePath = getWorkspacePath();
  }

  async start() {
    // Is config enabled?
    // TODO: add config to enable/disable watcher
    const isEnabled = getWorkspaceConfig("tuist.autogenerate");
    if (!isEnabled) {
      return new Disposable(() => {});
    }

    // Is project.swift exists?
    const workspacePath = getWorkspacePath();
    const isProjectExists = await isFileExists(path.join(workspacePath, "project.swift"));
    if (!isProjectExists) {
      commonLogger.log("project.swift not found, skipping tuist watcher", {
        workspacePath: getWorkspacePath(),
      });
      return new Disposable(() => {});
    }

    const swiftWatcher = vscode.workspace.createFileSystemWatcher("**/*.swift", false, true, false);
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
      tuistGenerateCommand()
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

  stop() {
    for (const watcher of this.watchers) {
      watcher.dispose();
    }
  }
}

export function createTuistWatcher(extension: ExtensionContext): vscode.Disposable {
  const watcher = new TuistGenWatcher(extension);
  void watcher.start();
  return new Disposable(() => watcher.stop());
}
