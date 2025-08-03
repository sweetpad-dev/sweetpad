import * as vscode from "vscode";
import type { ExtensionContext } from "../common/context.js";

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
