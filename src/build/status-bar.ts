import * as vscode from "vscode";
import type { ExtensionContext } from "../common/commands.js";

export class DefaultSchemeStatusBar {
  context: ExtensionContext;
  item: vscode.StatusBarItem;

  constructor(options: { context: ExtensionContext }) {
    this.context = options.context;
    const itemId = "sweetpad.build.statusBar";
    this.item = vscode.window.createStatusBarItem(itemId, vscode.StatusBarAlignment.Left, 0);
    this.item.name = "SweetPad: Current Scheme";
    this.item.command = "sweetpad.build.setDefaultScheme";
    this.item.tooltip = "Select the default Xcode scheme for building";
    void this.update();
    this.item.show();
    this.context.buildManager.on("defaultSchemeForBuildUpdated", () => {
      void this.update();
    });
  }

  update() {
    const scheme = this.context.buildManager.getDefaultSchemeForBuild();
    if (scheme) {
      this.item.text = `$(sweetpad-hexagons) ${scheme}`;
    } else {
      this.item.text = "$(sweetpad-help-hexagon) No default scheme";
    }
  }

  dispose() {
    this.item.dispose();
  }
}

export class BazelTargetStatusBar {
  context: ExtensionContext;
  item: vscode.StatusBarItem;

  constructor(options: { context: ExtensionContext }) {
    this.context = options.context;

    const itemId = "sweetpad.bazel.statusBar";
    this.item = vscode.window.createStatusBarItem(itemId, vscode.StatusBarAlignment.Left, -1);
    this.item.name = "SweetPad: Selected Bazel Target";
    this.item.tooltip = "Currently selected Bazel target";

    this.update();
    this.item.show();

    // Listen for target selection changes from BuildManager
    this.context.buildManager.on("selectedBazelTargetUpdated", () => {
      this.update();
    });
  }

  update() {
    const selectedTargetData = this.context.buildManager.getSelectedBazelTargetData();

    if (selectedTargetData) {
      const targetType = selectedTargetData.targetType;
      let icon = "$(package)";

      if (targetType === "test") {
        icon = "$(beaker)";
      } else if (targetType === "binary") {
        icon = "$(gear)";
      }

      this.item.text = `${icon} ${selectedTargetData.targetName}`;
      this.item.tooltip = `Selected Bazel Target: ${selectedTargetData.targetName} (${targetType})\nPackage: ${selectedTargetData.packageName}\nBuild: Ctrl+Shift+P → "Bazel Build Selected"`;
      this.item.command = "sweetpad.bazel.buildSelected"; // Allow clicking to build
    } else {
      this.item.text = "$(target) No Bazel target";
      this.item.tooltip = "No Bazel target selected";
      this.item.command = undefined;
    }
  }

  dispose() {
    this.item.dispose();
  }
}
