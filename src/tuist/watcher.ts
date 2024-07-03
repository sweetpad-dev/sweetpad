import * as vscode from 'vscode';
import { ExtensionContext } from '../common/commands';
import path from 'path';
import { getWorkspaceConfig } from "../common/config";
import { getWorkspacePath } from '../build/utils';
import { isFileExists } from '../common/files';
import { commonLogger } from '../common/logger';
import { Disposable } from 'vscode';
import { tuistGenerateCommand } from './command';

class TuistGenWatcher {
    private watchers: vscode.FileSystemWatcher[] = [];
    private throttle: NodeJS.Timeout | null = null;

    constructor(private extension: ExtensionContext) {}

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
    
        const swiftWatcher = vscode.workspace.createFileSystemWatcher("**/*.swift");
        swiftWatcher.onDidCreate((e) => this.handleChange(e));
        swiftWatcher.onDidDelete((e) => this.handleChange(e));
        this.watchers.push(swiftWatcher);
    
        commonLogger.log("tuist watcher started", {
          workspacePath: getWorkspacePath(),
        });
      }
    
      handleChange(e: vscode.Uri) {
        commonLogger.log("tuist watcher detected changes", {
          workspacePath: getWorkspacePath(),
          file: e.fsPath,
        });
        if (this.throttle) {
          clearTimeout(this.throttle);
        }
    
        this.throttle = setTimeout(() => {
          this.throttle = null;
          tuistGenerateCommand()
            .then(() => {
              commonLogger.log("tuist project was successfully generated", {
                workspacePath: getWorkspacePath(),
              });
            })
            .catch((error) => {
              commonLogger.error("Failed to generate tuist project", {
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

export function createTuistWatcher(extension: ExtensionContext): vscode.Disposable {
    const watcher = new TuistGenWatcher(extension);
    void watcher.start();
    return new Disposable(() => watcher.stop());
  }
  