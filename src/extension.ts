import * as vscode from "vscode";

import * as common from "./common/index.js";
import * as simulators from "./simulators/index.js";
import * as tools from "./tools/index.js";
import * as format from "./format/index.js";
import * as build from "./build/index.js";

export function activate(context: vscode.ExtensionContext) {
  // shortcut to push disposable to context.subscriptions
  const p = (disposable: vscode.Disposable) => context.subscriptions.push(disposable);

  // Preload exec function to avoid delay on first exec call
  void common.preloadExec();

  // Build
  // p(build.createTaskProvider());
  p(build.createTaskProvider());
  const buildTreeProvider = new build.BuildTreeProvider();
  p(vscode.window.registerTreeDataProvider("sweetpad.build.view", buildTreeProvider));
  p(vscode.commands.registerCommand("sweetpad.build.build", build.buildScheme));

  // Format
  p(format.createFormatStatusItem());
  p(format.createFormatProvider());
  p(vscode.commands.registerCommand("sweetpad.format", format.formatCommand));
  p(vscode.commands.registerCommand("sweetpad.format.showLogs", format.showLogsCommand));

  // Simulators
  const simulatorsTreeProvider = new simulators.SimulatorsTreeProvider();
  p(vscode.window.registerTreeDataProvider("sweetpad.simulators.view", simulatorsTreeProvider));
  p(vscode.commands.registerCommand("sweetpad.simulators.refresh", () => simulatorsTreeProvider.refresh()));
  p(vscode.commands.registerCommand("sweetpad.simulators.openSimulator", simulators.openSimulatorCommand));
  p(vscode.commands.registerCommand("sweetpad.simulators.removeCache", simulators.removeSimulatorCacheCommand));
  p(vscode.commands.registerCommand("sweetpad.simulators.start", simulators.startSimulatorCommand));
  p(vscode.commands.registerCommand("sweetpad.simulators.stop", simulators.stopSimulatorCommand));

  // Tools
  const toolsTreeProvider = new tools.ToolTreeProvider();
  p(vscode.window.registerTreeDataProvider("sweetpad.tools.view", toolsTreeProvider));
  p(vscode.commands.registerCommand("sweetpad.tools.install", tools.installToolCommand));
  p(vscode.commands.registerCommand("sweetpad.tools.refresh", () => toolsTreeProvider.refresh()));
  p(vscode.commands.registerCommand("sweetpad.tools.documentation", tools.openDocumentationCommand));
}

export function deactivate() {}
