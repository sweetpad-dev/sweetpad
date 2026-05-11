import * as vscode from "vscode";

import type { DestinationsManager } from "./manager";

export class DestinationStatusBar {
  private destinationsManager: DestinationsManager;
  item: vscode.StatusBarItem;

  constructor(options: { destinationsManager: DestinationsManager }) {
    this.destinationsManager = options.destinationsManager;
    const itemId = "sweetpad.destinations.statusBar";
    this.item = vscode.window.createStatusBarItem(itemId, vscode.StatusBarAlignment.Left, 0);
    this.item.name = "SweetPad: Current Destination";
    this.item.command = "sweetpad.destinations.select";
    this.item.tooltip = "Select destination for debugging";
  }

  async start(): Promise<void> {
    void this.update();
    this.item.show();
    this.destinationsManager.on("xcodeDestinationForBuildUpdated", () => {
      void this.update();
    });
  }

  update() {
    const destination = this.destinationsManager.getSelectedXcodeDestinationForBuild();
    if (destination) {
      this.item.text = `$(sweetpad-device-mobile-check) ${destination.name}`;
    } else {
      this.item.text = "$(sweetpad-device-mobile-question) No destination selected";
    }
  }

  dispose() {
    this.item.dispose();
  }
}
