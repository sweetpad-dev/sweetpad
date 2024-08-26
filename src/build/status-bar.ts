import * as vscode from "vscode";
import type { ExtensionContext } from "../common/commands.js";

export class DefaultSchemeStatusBar {
  context: ExtensionContext;
  item: vscode.StatusBarItem;

  constructor(options: { context: ExtensionContext }) {
    this.context = options.context;
    this.item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 0);
    this.item.command = "sweetpad.build.setDefaultScheme";
    this.item.tooltip = "Select the default Xcode scheme for building";
    void this.update();
    this.item.show();
    this.context.buildManager.on("defaultSchemeUpdated", () => {
      void this.update();
    });
  }

  update() {
    const scheme = this.context.buildManager.getDefaultScheme();
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
