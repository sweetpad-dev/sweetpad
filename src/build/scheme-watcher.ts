import path from "node:path";
import * as vscode from "vscode";
import { Disposable } from "vscode";
import type { ExtensionContext } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { isFileExists } from "../common/files";
import { commonLogger } from "../common/logger";
import { getWorkspacePath, prepareDerivedDataPath } from "./utils";

class SchemeWatcher {
  private watchers: vscode.FileSystemWatcher[] = [];
  // todo: rename to debounce, or make it behave like throttle
  private throttle: NodeJS.Timeout | null = null;
  private derivedDataPath: string | null;
  private workspacePath: string;

  constructor(private extension: ExtensionContext) {
    this.derivedDataPath = prepareDerivedDataPath();
    this.workspacePath = getWorkspacePath();
  }

  async start() {
    // Check if auto-refresh is enabled (default: true)
    const isEnabled = getWorkspaceConfig("build.autoRefreshSchemes") ?? true;
    if (!isEnabled) {
      commonLogger.log("Scheme auto-refresh is disabled", {
        workspacePath: this.workspacePath,
      });
      return new Disposable(() => {});
    }

    await this.setupWatchers();

    commonLogger.log("Scheme watcher started", {
      workspacePath: this.workspacePath,
    });
  }

  private async setupWatchers() {
    // Watch for Package.swift files (Swift Package Manager)
    const packageWatcher = vscode.workspace.createFileSystemWatcher(
      "**/Package.swift",
      false, // ignoreCreateEvents
      false, // ignoreChangeEvents
      false, // ignoreDeleteEvents
    );
    packageWatcher.onDidCreate((e) => this.handleChange(e, "Package.swift created"));
    packageWatcher.onDidChange((e) => this.handleChange(e, "Package.swift changed"));
    packageWatcher.onDidDelete((e) => this.handleChange(e, "Package.swift deleted"));
    this.watchers.push(packageWatcher);

    // Watch for .xcodeproj files
    const xcodeprojWatcher = vscode.workspace.createFileSystemWatcher(
      "**/*.xcodeproj",
      false, // ignoreCreateEvents
      true, // ignoreChangeEvents (these are directories)
      false, // ignoreDeleteEvents
    );
    xcodeprojWatcher.onDidCreate((e) => this.handleChange(e, ".xcodeproj created"));
    xcodeprojWatcher.onDidDelete((e) => this.handleChange(e, ".xcodeproj deleted"));
    this.watchers.push(xcodeprojWatcher);

    // Watch for .xcworkspace files
    const xcworkspaceWatcher = vscode.workspace.createFileSystemWatcher(
      "**/*.xcworkspace",
      false, // ignoreCreateEvents
      true, // ignoreChangeEvents (these are directories)
      false, // ignoreDeleteEvents
    );
    xcworkspaceWatcher.onDidCreate((e) => this.handleChange(e, ".xcworkspace created"));
    xcworkspaceWatcher.onDidDelete((e) => this.handleChange(e, ".xcworkspace deleted"));
    this.watchers.push(xcworkspaceWatcher);

    // Watch for scheme files (.xcscheme)
    const schemeWatcher = vscode.workspace.createFileSystemWatcher(
      "**/*.xcscheme",
      false, // ignoreCreateEvents
      false, // ignoreChangeEvents
      false, // ignoreDeleteEvents
    );
    schemeWatcher.onDidCreate((e) => this.handleChange(e, ".xcscheme created"));
    schemeWatcher.onDidChange((e) => this.handleChange(e, ".xcscheme changed"));
    schemeWatcher.onDidDelete((e) => this.handleChange(e, ".xcscheme deleted"));
    this.watchers.push(schemeWatcher);

    // Watch for project.yml files (XcodeGen)
    const workspacePath = getWorkspacePath();
    const isProjectYmlExists = await isFileExists(path.join(workspacePath, "project.yml"));
    if (isProjectYmlExists) {
      const projectYmlWatcher = vscode.workspace.createFileSystemWatcher(
        "**/project.yml",
        false, // ignoreCreateEvents
        false, // ignoreChangeEvents
        false, // ignoreDeleteEvents
      );
      projectYmlWatcher.onDidCreate((e) => this.handleChange(e, "project.yml created"));
      projectYmlWatcher.onDidChange((e) => this.handleChange(e, "project.yml changed"));
      projectYmlWatcher.onDidDelete((e) => this.handleChange(e, "project.yml deleted"));
      this.watchers.push(projectYmlWatcher);
    }

    // Watch for Tuist files (Project.swift, Workspace.swift)
    const isTuistProjectExists = await isFileExists(path.join(workspacePath, "Project.swift"));
    const isTuistWorkspaceExists = await isFileExists(path.join(workspacePath, "Workspace.swift"));

    if (isTuistProjectExists || isTuistWorkspaceExists) {
      const tuistWatcher = vscode.workspace.createFileSystemWatcher(
        "**/{Project,Workspace}.swift",
        false, // ignoreCreateEvents
        false, // ignoreChangeEvents
        false, // ignoreDeleteEvents
      );
      tuistWatcher.onDidCreate((e) => this.handleChange(e, "Tuist file created"));
      tuistWatcher.onDidChange((e) => this.handleChange(e, "Tuist file changed"));
      tuistWatcher.onDidDelete((e) => this.handleChange(e, "Tuist file deleted"));
      this.watchers.push(tuistWatcher);
    }

    // Watch for project.pbxproj files (changes inside .xcodeproj)
    const pbxprojWatcher = vscode.workspace.createFileSystemWatcher(
      "**/project.pbxproj",
      false, // ignoreCreateEvents
      false, // ignoreChangeEvents
      false, // ignoreDeleteEvents
    );
    pbxprojWatcher.onDidCreate((e) => this.handleChange(e, "project.pbxproj created"));
    pbxprojWatcher.onDidChange((e) => this.handleChange(e, "project.pbxproj changed"));
    pbxprojWatcher.onDidDelete((e) => this.handleChange(e, "project.pbxproj deleted"));
    this.watchers.push(pbxprojWatcher);
  }

  handleChange(e: vscode.Uri, reason: string) {
    commonLogger.log("Scheme watcher detected changes", {
      workspacePath: this.workspacePath,
      file: e.fsPath,
      reason: reason,
    });

    // Skip files in derived data path
    if (this.derivedDataPath && e.fsPath.startsWith(this.derivedDataPath)) {
      commonLogger.debug("Skipping file in derived data path", {
        file: e.fsPath,
        derivedDataPath: this.derivedDataPath,
      });
      return;
    }

    // Skip files in build output directories
    const buildPaths = ["/build/", "/.build/", "/DerivedData/"];
    if (buildPaths.some((buildPath) => e.fsPath.includes(buildPath))) {
      commonLogger.debug("Skipping file in build output directory", {
        file: e.fsPath,
      });
      return;
    }

    // Throttle the refresh to avoid excessive calls
    if (this.throttle) {
      clearTimeout(this.throttle);
    }

    // Get the refresh delay from config (default: 500ms)
    const refreshDelay = getWorkspaceConfig("build.autoRefreshSchemesDelay") ?? 500;

    this.throttle = setTimeout(() => {
      this.throttle = null;

      this.extension.buildManager
        .refreshSchemes()
        .then(() => {
          commonLogger.log("Schemes auto-refreshed successfully", {
            workspacePath: this.workspacePath,
            trigger: reason,
            file: path.basename(e.fsPath),
          });
        })
        .catch((error) => {
          commonLogger.error("Failed to auto-refresh schemes", {
            workspacePath: this.workspacePath,
            trigger: reason,
            file: e.fsPath,
            error: error,
          });
        });
    }, refreshDelay);
  }

  stop() {
    if (this.throttle) {
      clearTimeout(this.throttle);
      this.throttle = null;
    }

    for (const watcher of this.watchers) {
      watcher.dispose();
    }
    this.watchers = [];

    commonLogger.log("Scheme watcher stopped", {
      workspacePath: this.workspacePath,
    });
  }
}

export function createSchemeWatcher(extension: ExtensionContext): vscode.Disposable {
  const watcher = new SchemeWatcher(extension);
  void watcher.start();
  return new Disposable(() => watcher.stop());
}
