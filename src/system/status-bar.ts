import * as vscode from "vscode";
import { getWorkspaceConfig } from "../common/config";
import type { ExtensionContext } from "../common/context";

/**
 * Status bar item for showing what currently sweetpad is doing
 *
 * Usually, it's enough to call `update` method with the text you want to show
 * and if it in the context of a command, it will be automatically removed when
 * the command is finished. Otherwise, you need to call `remove` method to remove it
 * from the status bar manually.
 *
 * todo: think how to make it more robus, so that it not requires to listen to event
 */
export class ProgressStatusBar {
  context: ExtensionContext;
  statusBar: vscode.StatusBarItem;
  enabled = true;

  messageMapping: Map<string, string> = new Map();

  constructor(options: { context: ExtensionContext }) {
    this.context = options.context;
    // Status bar ID allows to separate the different status bar items from the same extension
    const statusBarId = "sweetpad.system.progressStatusBar";
    this.statusBar = vscode.window.createStatusBarItem(statusBarId, vscode.StatusBarAlignment.Left, 0);
    this.statusBar.command = "sweetpad.system.openTerminalPanel";
    this.statusBar.name = "SweetPad: Command Status";

    this.updateConfig();

    // Every time a command or task is finished we remove message of the current scope
    // and update the status bar accordingly
    this.context.on("executionScopeClosed", (scope) => {
      const scopeId = scope.id;
      if (!scopeId) return;

      this.messageMapping.delete(scopeId);
      this.displayBar();
    });

    this.context.on("workspaceConfigChanged", () => {
      this.updateConfig();
    });
  }

  dispose() {
    this.statusBar.dispose();
    this.messageMapping.clear();
  }

  updateText(text: string) {
    const scopeId = this.context.executionScope.getCurrentId();
    if (!scopeId) return;

    this.messageMapping.set(scopeId, text);
    this.displayBar();
  }

  updateConfig() {
    const enabled = getWorkspaceConfig("system.showProgressStatusBar") ?? true;
    if (this.enabled === enabled) {
      return; // nothing changed, no need to update
    }

    this.enabled = enabled;
    if (enabled) {
      // user enabled the status bar, show it if there are any messages
      this.displayBar();
    } else {
      // user disabled the status bar, hide it despite the messages
      this.statusBar.hide();
    }
  }

  displayBar() {
    if (!this.enabled) {
      return;
    }

    // No messages to show, hide the status bar for now
    if (this.messageMapping.size === 0) {
      this.statusBar.hide();
      return;
    }

    this.statusBar.show();
    // In simplest case, when we have only one message, we can show it directly in the status bar
    if (this.messageMapping.size === 1) {
      const text = this.messageMapping.values().next().value;
      this.statusBar.text = `$(gear~spin) ${text}...`;
      this.statusBar.tooltip = "Click to open terminal";
      return;
    }

    // In cases when we have multiple parallel commands running, we can show the status bar
    // with the number of commands running and a tooltip with the list of commands
    // that are running. Not idea, but better than nothing.
    this.statusBar.text = `$(gear~spin) ${this.messageMapping.size} commands running...`;
    const tooltip = new vscode.MarkdownString(
      `Active commands:\n${Array.from(this.messageMapping.values())
        .map((text) => `- ${text}...`)
        .join("\n")}\n`,
    );
    tooltip.isTrusted = true;
    this.statusBar.tooltip = tooltip;
    return;
  }
}
