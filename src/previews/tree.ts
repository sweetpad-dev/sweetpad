import * as vscode from "vscode";

import type { PreviewsManager } from "./manager.js";
import type { PreviewItem } from "./types.js";

/** File-level node grouping the previews declared in one Swift file. */
class PreviewFileTreeItem extends vscode.TreeItem {
  constructor(
    public readonly relativePath: string,
    public readonly previews: PreviewItem[],
  ) {
    super(relativePath.split("/").pop() ?? relativePath, vscode.TreeItemCollapsibleState.Expanded);
    this.description = relativePath;
    this.resourceUri = previews[0].uri;
    this.iconPath = vscode.ThemeIcon.File;
    this.contextValue = "preview-file";
  }
}

/** Leaf node for an individual preview; clicking it streams the preview. */
class PreviewTreeItem extends vscode.TreeItem {
  constructor(public readonly preview: PreviewItem) {
    super(preview.label, vscode.TreeItemCollapsibleState.None);
    this.description = `line ${preview.match.line + 1}`;
    this.iconPath = new vscode.ThemeIcon(preview.kind === "macro" ? "device-mobile" : "symbol-structure");
    this.contextValue = "preview-item";
    this.tooltip = preview.id;
    // Single click renders the preview (and reveals the source via the command).
    this.command = {
      title: "Preview in VSCode",
      command: "sweetpad.previews.render",
      arguments: [preview],
    };
  }
}

export class PreviewsTreeProvider implements vscode.TreeDataProvider<vscode.TreeItem> {
  private manager: PreviewsManager;

  private onDidChangeTreeDataEmitter = new vscode.EventEmitter<vscode.TreeItem | undefined | null | void>();
  readonly onDidChangeTreeData = this.onDidChangeTreeDataEmitter.event;

  constructor(options: { manager: PreviewsManager }) {
    this.manager = options.manager;
  }

  start(): void {
    this.manager.onDidChange(() => this.onDidChangeTreeDataEmitter.fire());
  }

  getTreeItem(element: vscode.TreeItem): vscode.TreeItem {
    return element;
  }

  async getChildren(element?: vscode.TreeItem): Promise<vscode.TreeItem[]> {
    if (!element) {
      const grouped = await this.manager.getGrouped();
      return grouped.map((group) => new PreviewFileTreeItem(group.relativePath, group.previews));
    }
    if (element instanceof PreviewFileTreeItem) {
      return element.previews.map((preview) => new PreviewTreeItem(preview));
    }
    return [];
  }
}
