import * as vscode from "vscode";

import { BuildTreeProvider } from "./build/tree.js";
import {
  launchCommand,
  buildCommand,
  cleanCommand,
  generateBuildServerConfigCommand,
  openXcodeCommand,
  removeBundleDirCommand,
  resolveDependenciesCommand,
} from "./build/commands.js";
import { formatCommand, showLogsCommand } from "./format/commands.js";
import { createFormatStatusItem } from "./format/status.js";
import { createFormatProvider } from "./format/provider.js";
import {
  openSimulatorCommand,
  removeSimulatorCacheCommand,
  startSimulatorCommand,
  stopSimulatorCommand,
} from "./simulators/commands.js";
import { SimulatorsTreeProvider } from "./simulators/tree.js";
import { ToolTreeProvider } from "./tools/tree.js";
import { installToolCommand, openDocumentationCommand } from "./tools/commands.js";
import { CommandExecution, ExtensionContext } from "./common/commands.js";
import { selectXcodeWorkspaceCommand } from "./build/commands.js";
import { resetSweetpadCache } from "./system/commands.js";
import { XcodeBuildTaskProvider } from "./build/provider.js";

export function activate(context: vscode.ExtensionContext) {
  // Trees 🎄
  const simulatorsTreeProvider = new SimulatorsTreeProvider();
  const buildTreeProvider = new BuildTreeProvider();
  const toolsTreeProvider = new ToolTreeProvider();

  const _context = new ExtensionContext({
    context: context,
    buildProvider: buildTreeProvider,
    simulatorsProvider: simulatorsTreeProvider,
    toolsProvider: toolsTreeProvider,
  });

  // shortcut to push disposable to context.subscriptions
  const p = (disposable: vscode.Disposable) => context.subscriptions.push(disposable);

  function registerCommand(command: string, callback: (context: CommandExecution, ...args: any[]) => Promise<void>) {
    return vscode.commands.registerCommand(command, (...args: any[]) => {
      const execution = new CommandExecution(command, callback, _context);
      return execution.run(...args);
    });
  }

  const buildTaskProvider = new XcodeBuildTaskProvider(_context);

  // Tasks
  p(vscode.tasks.registerTaskProvider(buildTaskProvider.type, buildTaskProvider));

  // Build
  p(vscode.window.registerTreeDataProvider("sweetpad.build.view", buildTreeProvider));
  p(registerCommand("sweetpad.build.refresh", async () => buildTreeProvider.refresh()));
  p(registerCommand("sweetpad.build.launch", launchCommand));
  p(registerCommand("sweetpad.build.build", buildCommand));
  p(registerCommand("sweetpad.build.clean", cleanCommand));
  p(registerCommand("sweetpad.build.resolveDependencies", resolveDependenciesCommand));
  p(registerCommand("sweetpad.build.removeBundleDir", removeBundleDirCommand));
  p(registerCommand("sweetpad.build.genereateBuildServerConfig", generateBuildServerConfigCommand));
  p(registerCommand("sweetpad.build.openXcode", openXcodeCommand));
  p(registerCommand("sweetpad.build.selectXcodeWorkspace", selectXcodeWorkspaceCommand));

  // Format
  p(createFormatStatusItem());
  p(createFormatProvider());
  p(registerCommand("sweetpad.format.run", formatCommand));
  p(registerCommand("sweetpad.format.showLogs", showLogsCommand));

  // Simulators
  p(vscode.window.registerTreeDataProvider("sweetpad.simulators.view", simulatorsTreeProvider));
  p(registerCommand("sweetpad.simulators.refresh", async () => simulatorsTreeProvider.refresh()));
  p(registerCommand("sweetpad.simulators.openSimulator", openSimulatorCommand));
  p(registerCommand("sweetpad.simulators.removeCache", removeSimulatorCacheCommand));
  p(registerCommand("sweetpad.simulators.start", startSimulatorCommand));
  p(registerCommand("sweetpad.simulators.stop", stopSimulatorCommand));

  // Tools
  p(vscode.window.registerTreeDataProvider("sweetpad.tools.view", toolsTreeProvider));
  p(registerCommand("sweetpad.tools.install", installToolCommand));
  p(registerCommand("sweetpad.tools.refresh", async () => toolsTreeProvider.refresh()));
  p(registerCommand("sweetpad.tools.documentation", openDocumentationCommand));

  // System
  p(registerCommand("sweetpad.system.resetSweetpadCache", resetSweetpadCache));
}

export function deactivate() {}
