import path from "node:path";
import { Disposable } from "vscode";
import * as vscode from "vscode";
import { getWorkspacePath } from "../build/utils";
import type { ExtensionContext } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { isFileExists } from "../common/files";
import { commonLogger } from "../common/logger";
import { xcodgenGenerateCommand } from "./commands";

class XcodeGenWatcher {
  private watchers: vscode.FileSystemWatcher[] = [];
  private throttle: NodeJS.Timeout | null = null;
  private derivedDataPath: string | null;
  private workspacePath: string;

  constructor(private extension: ExtensionContext) {
    this.derivedDataPath = null;
    this.workspacePath = getWorkspacePath();
  }

  async start() {
    // Is config enabled?
    // TODO: add config to enable/disable watcher
    const isEnabled = getWorkspaceConfig("xcodegen.autogenerate");
    if (!isEnabled) {
      return new Disposable(() => {});
    }

    // Is project.yml exists?
    const workspacePath = getWorkspacePath();
    const isProjectExists = await isFileExists(path.join(workspacePath, "project.yml"));
    if (!isProjectExists) {
      commonLogger.log("project.yml not found, skipping xcodegen watcher", {
        workspacePath: getWorkspacePath(),
      });
      return new Disposable(() => {});
    }

    const swiftWatcher = vscode.workspace.createFileSystemWatcher("**/*.swift", false, true, false);
    swiftWatcher.onDidCreate((e) => this.handleChange(e));
    swiftWatcher.onDidDelete((e) => this.handleChange(e));
    this.watchers.push(swiftWatcher);

    commonLogger.log("XcodeGen watcher started", {
      workspacePath: this.workspacePath,
    });
  }

  handleChange(e: vscode.Uri) {
    commonLogger.log("XcodeGen watcher detected changes", {
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
      xcodgenGenerateCommand()
        .then(() => {
          commonLogger.log("XcodeGen project was successfully generated", {
            workspacePath: this.workspacePath,
          });
        })
        .catch((error) => {
          commonLogger.error("Failed to generate XcodeGen project", {
            workspacePath: this.workspacePath,
            error,
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

export function createXcodeGenWatcher(extension: ExtensionContext): vscode.Disposable {
  const watcher = new XcodeGenWatcher(extension);
  void watcher.start();
  return new Disposable(() => watcher.stop());
}
