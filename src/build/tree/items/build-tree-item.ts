import path from "node:path";
import * as vscode from "vscode";

// Forward declaration to avoid circular dependency
export interface IBuildTreeProvider {
  defaultWorkspacePath?: string;
  defaultSchemeForBuild?: string;
  defaultSchemeForTesting?: string;
  buildManager: {
    setDefaultSchemeForBuild(scheme: string): void;
    setDefaultSchemeForTesting(scheme: string): void;
  };
}

export class BuildTreeItem extends vscode.TreeItem {
  public provider: IBuildTreeProvider;
  public scheme: string;
  public workspacePath: string;

  constructor(options: {
    scheme: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    provider: IBuildTreeProvider;
    workspacePath?: string;
  }) {
    super(options.scheme, options.collapsibleState);
    this.provider = options.provider;
    this.scheme = options.scheme;
    this.workspacePath = options.workspacePath || this.provider.defaultWorkspacePath || "";

    const color = new vscode.ThemeColor("sweetpad.scheme");
    this.iconPath = new vscode.ThemeIcon("sweetpad-package", color);
    this.contextValue = "sweetpad.build.view.item";

    let description = "";
    // Only show checkmark if this is the default scheme for this specific workspace
    if (
      this.scheme === this.provider.defaultSchemeForBuild &&
      this.workspacePath === this.provider.defaultWorkspacePath
    ) {
      description = `${description} ✓`;
    }
    if (this.scheme === this.provider.defaultSchemeForTesting) {
      description = `${description} (t)`;
    }
    if (description) {
      this.description = description;
    }

    // Add workspace name to tooltip for clarity
    if (this.workspacePath) {
      const workspaceName = path.basename(this.workspacePath);
      this.tooltip = `Scheme: ${this.scheme}\nWorkspace: ${workspaceName}`;

      // Add workspace info to the label for clarity
      this.description = `${description || ""} (${workspaceName})`.trim();
    } else {
      this.tooltip = `Scheme: ${this.scheme}`;
    }

    // Store command with the correct arguments that point to this specific scheme and workspace
    this.command = {
      command: "sweetpad.build.launch",
      title: "Launch",
      arguments: [this],
    };
  }
}
