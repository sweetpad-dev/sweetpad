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
import { ExtensionContext } from "./common/commands.js";
import { selectXcodeWorkspaceCommand } from "./build/commands.js";
import { createIssueGenericCommand, createIssueNoSchemesCommand, resetSweetpadCache } from "./system/commands.js";
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
  const d = _context.disposable.bind(_context);
  const command = _context.registerCommand.bind(_context);

  const buildTaskProvider = new XcodeBuildTaskProvider(_context);

  // Tasks
  d(vscode.tasks.registerTaskProvider(buildTaskProvider.type, buildTaskProvider));

  // Build
  d(vscode.window.registerTreeDataProvider("sweetpad.build.view", buildTreeProvider));
  d(command("sweetpad.build.refresh", async () => buildTreeProvider.refresh()));
  d(command("sweetpad.build.launch", launchCommand));
  d(command("sweetpad.build.build", buildCommand));
  d(command("sweetpad.build.clean", cleanCommand));
  d(command("sweetpad.build.resolveDependencies", resolveDependenciesCommand));
  d(command("sweetpad.build.removeBundleDir", removeBundleDirCommand));
  d(command("sweetpad.build.genereateBuildServerConfig", generateBuildServerConfigCommand));
  d(command("sweetpad.build.openXcode", openXcodeCommand));
  d(command("sweetpad.build.selectXcodeWorkspace", selectXcodeWorkspaceCommand));

  // Format
  d(createFormatStatusItem());
  d(createFormatProvider());
  d(command("sweetpad.format.run", formatCommand));
  d(command("sweetpad.format.showLogs", showLogsCommand));

  // Simulators
  d(vscode.window.registerTreeDataProvider("sweetpad.simulators.view", simulatorsTreeProvider));
  d(command("sweetpad.simulators.refresh", async () => simulatorsTreeProvider.refresh()));
  d(command("sweetpad.simulators.openSimulator", openSimulatorCommand));
  d(command("sweetpad.simulators.removeCache", removeSimulatorCacheCommand));
  d(command("sweetpad.simulators.start", startSimulatorCommand));
  d(command("sweetpad.simulators.stop", stopSimulatorCommand));

  // Tools
  d(vscode.window.registerTreeDataProvider("sweetpad.tools.view", toolsTreeProvider));
  d(command("sweetpad.tools.install", installToolCommand));
  d(command("sweetpad.tools.refresh", async () => toolsTreeProvider.refresh()));
  d(command("sweetpad.tools.documentation", openDocumentationCommand));

  // System
  d(command("sweetpad.system.resetSweetpadCache", resetSweetpadCache));
  d(command("sweetpad.system.createIssue.generic", createIssueGenericCommand));
  d(command("sweetpad.system.createIssue.noSchemes", createIssueNoSchemesCommand));
}

export function deactivate() {}
