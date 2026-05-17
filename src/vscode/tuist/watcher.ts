import path from "node:path";

import * as vscode from "vscode";

import { prepareDerivedDataPath } from "../../core/build/utils";
import type { ConfigProvider } from "../../core/config/types";
import { isFileExists } from "../../core/files";
import type { Logger } from "../../core/logger/types";
import type { WorkspaceRoot } from "../../core/workspace-root";

export class TuistGenWatcher implements vscode.Disposable {
  private watchers: vscode.FileSystemWatcher[] = [];
  private throttle: NodeJS.Timeout | null = null;
  private derivedDataPath: string | null = null;
  private workspacePath = "";
  private workspaceRoot: WorkspaceRoot;
  private config: ConfigProvider;
  private logger: Logger;

  constructor(options: { workspaceRoot: WorkspaceRoot; config: ConfigProvider; logger: Logger }) {
    this.workspaceRoot = options.workspaceRoot;
    this.config = options.config;
    this.logger = options.logger;
  }

  async start(): Promise<void> {
    this.workspacePath = this.workspaceRoot.getPath();
    this.derivedDataPath = prepareDerivedDataPath({ config: this.config, cwd: this.workspacePath });
    // Is config enabled?
    // TODO: add config to enable/disable watcher
    const isEnabled = this.config.get("tuist.autogenerate");
    if (!isEnabled) {
      return;
    }

    // We don't even need to start the watcher if there is no tuist files in the workspace
    const isTuistFileExists =
      (await isFileExists(path.join(this.workspacePath, "Project.swift"))) ||
      (await isFileExists(path.join(this.workspacePath, "Workspace.swift")));
    if (!isTuistFileExists) {
      this.logger.log("Project.swift or Workspace.swift not found, skipping tuist watcher", {
        workspacePath: this.workspacePath,
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

    this.logger.log("tuist watcher started", {
      workspacePath: this.workspacePath,
    });
  }

  handleChange(e: vscode.Uri) {
    this.logger.log("tuist watcher detected changes", {
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
          this.logger.log("tuist project was successfully generated", {
            workspacePath: this.workspacePath,
          });
        })
        .catch((error) => {
          this.logger.error("Failed to generate tuist project", {
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
