import * as vscode from "vscode";

import type { BuildManager } from "../../core/build/manager";

export class DefaultSchemeStatusBar {
  private buildManager: BuildManager;
  item: vscode.StatusBarItem;

  constructor(options: { buildManager: BuildManager }) {
    this.buildManager = options.buildManager;
    const itemId = "sweetpad.build.statusBar";
    this.item = vscode.window.createStatusBarItem(itemId, vscode.StatusBarAlignment.Left, 0);
    this.item.name = "SweetPad: Current Scheme";
    this.item.command = "sweetpad.build.setDefaultScheme";
    this.item.tooltip = "Select the default Xcode scheme for building";
  }

  async start(): Promise<void> {
    void this.update();
    this.item.show();
    this.buildManager.on("defaultSchemeForBuildUpdated", () => {
      void this.update();
    });
  }

  update() {
    const scheme = this.buildManager.getDefaultSchemeForBuild();
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
