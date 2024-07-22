import * as vscode from "vscode";
import { ExtensionContext } from "../common/commands.js";

export class SchemeStatusBar {
  context: ExtensionContext;
  item: vscode.StatusBarItem;

  constructor(options: { context: ExtensionContext }) {
    this.context = options.context;
    this.item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 0);
    this.item.command = "sweetpad.build.selectXcodeScheme";
    this.item.tooltip = "Select default scheme for the project";
    void this.update();
    this.item.show();
    this.context.buildManager.on("selectedSchemeUpdated", () => {
      void this.update();
    });
  }

  update() {
    const scheme = this.context.buildManager.getSelectedScheme();
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
