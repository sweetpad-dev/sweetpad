import { ExtensionContext } from "../common/commands";
import { Disposable } from "vscode";
import * as vscode from "vscode";
import { getWorkspaceConfig } from "../common/config";
import { isFileExists } from "../common/files";
import { commonLogger } from "../common/logger";
import { getWorkspacePath } from "../build/utils";
import { xcodgenGenerateCommand } from "./commands";
import path from "path";

class XcodeGenWatcher {
  private watchers: vscode.FileSystemWatcher[] = [];
  private throttle: NodeJS.Timeout | null = null;

  constructor(private extension: ExtensionContext) {}

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

    const swiftWatcher = vscode.workspace.createFileSystemWatcher("**/*.swift");
    swiftWatcher.onDidCreate((e) => this.handleChange(e));
    swiftWatcher.onDidDelete((e) => this.handleChange(e));
    this.watchers.push(swiftWatcher);

    commonLogger.log("XcodeGen watcher started", {
      workspacePath: getWorkspacePath(),
    });
  }

  handleChange(e: vscode.Uri) {
    commonLogger.log("XcodeGen watcher detected changes", {
      workspacePath: getWorkspacePath(),
      file: e.fsPath,
    });
    if (this.throttle) {
      clearTimeout(this.throttle);
    }

    this.throttle = setTimeout(() => {
      this.throttle = null;
      xcodgenGenerateCommand()
        .then(() => {
          commonLogger.log("XcodeGen project was successfully generated", {
            workspacePath: getWorkspacePath(),
          });
        })
        .catch((error) => {
          commonLogger.error("Failed to generate XcodeGen project", {
            workspacePath: getWorkspacePath(),
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
