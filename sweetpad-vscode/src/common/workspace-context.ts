import * as events from "node:events";

import * as vscode from "vscode";

import { ExtensionError } from "./errors";
import { commonLogger } from "./logger";

/**
 * Owns the VS Code workspace folder SweetPad operates on.
 *
 * SweetPad works on one Xcode project at a time, and in a multi-root workspace that project may
 * live in any folder, so this remembers the folder holding the selected xcworkspace and every
 * "workspace root" lookup resolves against it rather than defaulting to the first folder.
 *
 * The resolved folder moves for two independent reasons — a project is selected in another folder,
 * or the window's folder list changes underneath it — so both report through one place and
 * subscribers see a single deduplicated event.
 */
export class WorkspaceContextService implements vscode.Disposable {
  private activeFolder: string | undefined;
  /** The folder last reported, so the two reasons it can move dedupe against each other. */
  private notifiedFolder: string | undefined;
  private readonly emitter = new events.EventEmitter<{ changed: [folder: string] }>();
  private foldersSubscription: vscode.Disposable | undefined;

  /**
   * The folder SweetPad currently operates on, or undefined when the window has no folder open.
   * The remembered folder only wins while it is still one of the window's folders.
   */
  resolve(): string | undefined {
    const folders = vscode.workspace.workspaceFolders;
    if (!folders || folders.length === 0) {
      return undefined;
    }
    if (this.activeFolder && folders.some((folder) => folder.uri.fsPath === this.activeFolder)) {
      return this.activeFolder;
    }
    return folders[0].uri.fsPath;
  }

  /**
   * The folder SweetPad operates on right now. Only for callers with no project in hand — the
   * discovery-index registration, the BSP socket, the watchers.
   *
   * Anything working on a specific project resolves its own root next to the xcworkspace
   * (`getWorkspaceFolderForPath(xcworkspace) ?? root`) and threads that value, because this one
   * moves whenever another project is selected: two reads inside one build can otherwise disagree,
   * writing the build server config to one folder while xcodebuild runs in another.
   */
  get root(): string {
    const folder = this.resolve();
    if (!folder) {
      throw new ExtensionError("No workspace folder found");
    }
    return folder;
  }

  /**
   * Remember the workspace folder holding the given xcworkspace, so the root resolves against it.
   * No-op when the path is outside every folder (e.g. a git worktree next to the repo).
   */
  setActiveFolder(xcworkspacePath: string): void {
    const folder = vscode.workspace.getWorkspaceFolder(vscode.Uri.file(xcworkspacePath))?.uri.fsPath;
    if (!folder || folder === this.activeFolder) {
      return;
    }
    this.activeFolder = folder;
    commonLogger.log("Active workspace folder changed", {
      folder: folder,
      xcworkspace: xcworkspacePath,
    });
    this.notifyChanged();
  }

  /**
   * Subscribe to the root moving to another folder.
   *
   * Anything deriving long-lived state from the root — a socket path, a key in the discovery index,
   * a file watcher — holds a value that is only right for one folder, and has to rebuild it when
   * the folder moves.
   */
  onDidChange(listener: (folder: string) => void): vscode.Disposable {
    this.emitter.on("changed", listener);
    return {
      dispose: () => {
        this.emitter.off("changed", listener);
      },
    };
  }

  /**
   * Begin reporting the root moving because the window's folder list changed. Removing the folder
   * that holds the current project drops the root back to the first folder, which is a move no
   * call to `setActiveFolder` announces.
   */
  start(): void {
    this.notifiedFolder = this.resolve();
    this.foldersSubscription = vscode.workspace.onDidChangeWorkspaceFolders(() => this.notifyChanged());
  }

  dispose(): void {
    this.foldersSubscription?.dispose();
    this.foldersSubscription = undefined;
    this.emitter.removeAllListeners("changed");
  }

  private notifyChanged(): void {
    const folder = this.resolve();
    if (folder && folder !== this.notifiedFolder) {
      this.notifiedFolder = folder;
      this.emitter.emit("changed", folder);
    }
  }
}
