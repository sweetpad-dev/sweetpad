import * as vscode from "vscode";
import type { BazelTarget, BazelPackage } from "../../utils";
import type { SelectedBazelTargetData } from "../../manager";

// Forward declaration to avoid circular dependency
export interface IBazelTreeProvider {
  getSelectedBazelTargetData(): SelectedBazelTargetData | undefined;
}

export class BazelTreeItem extends vscode.TreeItem {
  public provider: IBazelTreeProvider;
  public target: BazelTarget;
  public package: BazelPackage;
  public workspacePath: string;
  
  constructor(options: {
    target: BazelTarget;
    package: BazelPackage;
    provider: IBazelTreeProvider;
    workspacePath: string;
  }) {
    // Validate required properties
    if (!options.target) {
      throw new Error("BazelTreeItem requires target");
    }
    if (!options.target.name) {
      throw new Error("BazelTreeItem target requires name");
    }
    if (!options.package) {
      throw new Error("BazelTreeItem requires package");
    }
    if (!options.workspacePath || typeof options.workspacePath !== 'string') {
      throw new Error("BazelTreeItem requires valid workspacePath string");
    }

    super(options.target.name, vscode.TreeItemCollapsibleState.None);
    this.provider = options.provider;
    this.target = options.target;
    this.package = options.package;
    this.workspacePath = options.workspacePath;

    // Check if this target is currently selected by comparing build labels
    const selectedTargetData = this.provider.getSelectedBazelTargetData();
    const isSelected = selectedTargetData?.buildLabel === this.target.buildLabel;
    
    // Set icon based on target type and selection state
    const color = new vscode.ThemeColor("sweetpad.scheme");
    
    if (this.target.type === "test") {
      this.iconPath = new vscode.ThemeIcon("beaker", color);
    } else {
      this.iconPath = new vscode.ThemeIcon("package", color);
    }
    
    this.contextValue = "sweetpad.bazel.target";
    
    // Add type, package info, and selection indicator to description
    let description = `${this.target.type} • ${this.package.name}`;
    if (isSelected) {
      description = `${description} ✓`; // Add checkmark for selected target
    }
    this.description = description;
    
    // Set tooltip with build and test commands
    let tooltip = `Target: ${this.target.name}\nType: ${this.target.type}\nPackage: ${this.package.name}`;
    tooltip += `\nBuild: bazel build ${this.target.buildLabel}`;
    if (this.target.testLabel) {
      tooltip += `\nTest: bazel test ${this.target.testLabel}`;
    }
    if (isSelected) {
      tooltip += `\n\n✓ Currently selected target`;
    }
    this.tooltip = tooltip;
    
    // Set command to select the target - pass only serializable data
    this.command = {
      command: 'sweetpad.bazel.selectTarget',
      title: 'Select Bazel Target',
      arguments: [{
        buildLabel: this.target.buildLabel,
        workspacePath: this.workspacePath
      }]
    };
  }
}
