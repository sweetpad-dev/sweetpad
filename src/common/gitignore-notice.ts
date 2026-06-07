import { execFile } from "node:child_process";
import { promises as fs } from "node:fs";
import * as path from "node:path";
import { promisify } from "node:util";

import * as vscode from "vscode";

import { SWEETPAD_DIR_NAME } from "../cli-server/paths";
import { commonLogger } from "./logger";
import { WorkspaceStateService } from "./workspace-state";

const execFileAsync = promisify(execFile);

/**
 * Watches for the project-local `.sweetpad/` directory and, the first time it
 * appears in a git repo without being ignored, offers to add it to .gitignore.
 * `.sweetpad/` is ephemeral runtime state (sockets, build history) and must not
 * be committed. Honors a "Don't ask again" per-workspace.
 */
export class GitignoreNotifier implements vscode.Disposable {
  private readonly workspacePath: string;
  private readonly workspaceState: WorkspaceStateService;
  private readonly watcher: vscode.FileSystemWatcher;
  private resolved = false;
  private inFlight = false;

  constructor(options: { workspacePath: string; workspaceState: WorkspaceStateService }) {
    this.workspacePath = options.workspacePath;
    this.workspaceState = options.workspaceState;

    const pattern = new vscode.RelativePattern(this.workspacePath, SWEETPAD_DIR_NAME);
    this.watcher = vscode.workspace.createFileSystemWatcher(pattern, false, true, true);
    this.watcher.onDidCreate(() => void this.check());
    void this.check();
  }

  private async check(): Promise<void> {
    if (this.resolved || this.inFlight) return;
    this.inFlight = true;
    try {
      if (this.workspaceState.get("gitignore.dismissed")) {
        this.resolved = true;
        return;
      }
      if (!(await this.dirExists(path.join(this.workspacePath, SWEETPAD_DIR_NAME)))) {
        return; // not there yet; a later create event will re-check
      }
      if (!(await this.isGitRepo(this.workspacePath))) {
        this.resolved = true; // nothing to gitignore
        return;
      }
      if (await this.isIgnored(this.workspacePath, SWEETPAD_DIR_NAME)) {
        this.resolved = true;
        return;
      }
      this.resolved = true;
      await this.offer();
    } catch (err) {
      commonLogger.debug("gitignore check failed", { error: err });
    } finally {
      this.inFlight = false;
    }
  }

  private async offer(): Promise<void> {
    const add = "Add to .gitignore";
    const never = "Don't ask again";
    const choice = await vscode.window.showInformationMessage(
      "SweetPad created a .sweetpad/ folder for runtime state (sockets, build history). Add it to .gitignore?",
      add,
      "Not now",
      never,
    );
    if (choice === add) {
      await this.appendGitignore(this.workspacePath, SWEETPAD_DIR_NAME);
    } else if (choice === never) {
      this.workspaceState.update("gitignore.dismissed", true);
    }
  }

  private async dirExists(p: string): Promise<boolean> {
    try {
      return (await fs.stat(p)).isDirectory();
    } catch {
      return false;
    }
  }

  private async isGitRepo(cwd: string): Promise<boolean> {
    try {
      await execFileAsync("git", ["rev-parse", "--is-inside-work-tree"], { cwd });
      return true;
    } catch {
      return false;
    }
  }

  // `git check-ignore` is authoritative: it honors nested .gitignore files, parent
  // globs, .git/info/exclude, and the global excludesfile. Exit 0 means ignored.
  private async isIgnored(cwd: string, target: string): Promise<boolean> {
    try {
      await execFileAsync("git", ["check-ignore", "--quiet", "--", target], { cwd });
      return true;
    } catch {
      return false;
    }
  }

  private async appendGitignore(workspacePath: string, entry: string): Promise<void> {
    const file = path.join(workspacePath, ".gitignore");
    let existing = "";
    try {
      existing = await fs.readFile(file, "utf8");
    } catch {
      // new .gitignore
    }
    const lines = new Set(existing.split("\n").map((l) => l.trim()));
    if (lines.has(`${entry}/`) || lines.has(entry)) return;
    const prefix = existing.length > 0 && !existing.endsWith("\n") ? "\n" : "";
    await fs.appendFile(file, `${prefix}${entry}/\n`);
  }

  dispose(): void {
    this.watcher.dispose();
  }
}
