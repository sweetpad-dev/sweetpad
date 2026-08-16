import * as vscode from "vscode";

import { prepareDerivedDataPath, workspaceFoldersContaining } from "../build/utils";
import { getWorkspaceConfig } from "../common/config";
import { commonLogger } from "../common/logger";
import type { WorkspaceContextService } from "../common/workspace-context";

export class XcodeGenWatcher implements vscode.Disposable {
  private watchers: vscode.FileSystemWatcher[] = [];
  // One pending generate per folder: a change in one project must not cancel another's.
  private throttles = new Map<string, NodeJS.Timeout>();
  private derivedDataPath: string | null = null;
  private foldersSubscription: vscode.Disposable | undefined;
  private disposed = false;
  private queue: Promise<void> = Promise.resolve();
  private readonly workspaceContext: WorkspaceContextService;

  constructor(options: { workspaceContext: WorkspaceContextService }) {
    this.workspaceContext = options.workspaceContext;
  }

  async start(): Promise<void> {
    // Each watcher below is scoped to one folder, so the set only covers the folders open when it
    // was built. Rebuild it whenever the window's folders change: otherwise a project.yml added
    // after activation is never watched, and a folder that leaves keeps a watcher that can still
    // fire a generate in a directory no longer in the window.
    this.foldersSubscription = vscode.workspace.onDidChangeWorkspaceFolders(() => {
      void this.rebuild().catch((error) => {
        commonLogger.error("Failed to rebuild XcodeGen watchers", { error: error });
      });
    });
    await this.rebuild();
  }

  /**
   * Queued rather than run concurrently: two folder changes in quick succession would otherwise
   * both build a set of watchers, and only the last set assigned would ever be disposed.
   */
  private rebuild(): Promise<void> {
    const run = this.queue.then(
      () => this.applyWatchers(),
      () => this.applyWatchers(),
    );
    // The queue itself must never stay rejected, or one failed rebuild wedges every later one.
    this.queue = run.catch(() => {});
    return run;
  }

  private async applyWatchers(): Promise<void> {
    if (this.disposed) return;
    this.derivedDataPath = prepareDerivedDataPath({ workspaceRoot: this.workspaceContext.root });

    // Is config enabled?
    // TODO: add config to enable/disable watcher
    const isEnabled = getWorkspaceConfig("xcodegen.autogenerate");
    // Every folder holding a project.yml gets its own watcher, scoped to that folder, so the
    // generate runs in the directory whose spec triggered it.
    const folders = isEnabled ? await workspaceFoldersContaining("project.yml") : [];
    if (this.disposed) return;

    this.disposeWatchers();
    if (folders.length === 0) {
      commonLogger.log("No project.yml in any workspace folder, xcodegen watcher idle");
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

    commonLogger.log("XcodeGen watcher started", {
      roots: folders.map((folder) => folder.uri.fsPath),
    });
  }

  handleChange(root: string, e: vscode.Uri) {
    commonLogger.log("XcodeGen watcher detected changes", {
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
        Promise.resolve(vscode.commands.executeCommand("sweetpad.xcodegen.generate", root))
          .then(() => {
            commonLogger.log("XcodeGen generate finished", {
              root: root,
            });
          })
          .catch((error) => {
            commonLogger.error("Failed to generate XcodeGen project", {
              root: root,
              error: error,
            });
          });
      }, 1000 /* 1s */),
    );
  }

  /** Drops the current watchers and any generate they had pending. */
  private disposeWatchers(): void {
    for (const timeout of this.throttles.values()) {
      clearTimeout(timeout);
    }
    this.throttles.clear();
    for (const watcher of this.watchers) {
      watcher.dispose();
    }
    this.watchers = [];
  }

  dispose(): void {
    this.disposed = true;
    this.foldersSubscription?.dispose();
    this.foldersSubscription = undefined;
    this.disposeWatchers();
  }
}
