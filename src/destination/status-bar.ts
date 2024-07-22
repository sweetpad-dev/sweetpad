import * as vscode from "vscode";
import { ExtensionContext } from "../common/commands.js";

export class DestinationStatusBar {
  context: ExtensionContext;
  item: vscode.StatusBarItem;

  constructor(options: { context: ExtensionContext }) {
    this.context = options.context;
    this.item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 0);
    this.item.command = "sweetpad.destinations.select";
    this.item.tooltip = "Select destination for debugging";
    void this.update();
    this.item.show();
    this.context.destinationsManager.on("xcodeDestinationUpdated", () => {
      void this.update();
    });
  }

  update() {
    const destination = this.context.destinationsManager.getSelectedXcodeDestination();
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
