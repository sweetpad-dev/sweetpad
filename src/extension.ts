import * as vscode from "vscode";
import * as http from 'http';
import type { Express } from 'express'; // Fix the import to use type import
import {
  buildAndPeripheryScanCommand,
  buildCommand,
  cleanCommand,
  createPeripheryConfigCommand,
  debuggingBuildCommand,
  debuggingLaunchCommand,
  debuggingRunCommand,
  diagnoseBuildSetupCommand,
  generateBuildServerConfigCommand,
  launchCommand,
  openXcodeCommand,
  peripheryScanCommand,
  removeBundleDirCommand,
  resolveDependenciesCommand,
  runCommand,
  selectConfigurationForBuildCommand,
  selectXcodeSchemeForBuildCommand,
  selectXcodeWorkspaceCommand,
  testCommand,
  testWithSwiftTestingCommand,
} from "./build/commands.js";
import { BuildManager } from "./build/manager.js";
import { XcodeBuildTaskProvider } from "./build/provider.js";
import { DefaultSchemeStatusBar } from "./build/status-bar.js";
import { WorkspaceTreeProvider } from "./build/tree.js";
import { ExtensionContext } from "./common/commands.js";
import { errorReporting } from "./common/error-reporting.js";
import { Logger, commonLogger } from "./common/logger.js";
import { getAppPathCommand } from "./debugger/commands.js";
import { registerDebugConfigurationProvider } from "./debugger/provider.js";
import {
  removeRecentDestinationCommand,
  selectDestinationForBuildCommand,
  selectDestinationForTestingCommand,
} from "./destination/commands.js";
import { DestinationsManager } from "./destination/manager.js";
import { DestinationStatusBar } from "./destination/status-bar.js";
import { DestinationsTreeProvider } from "./destination/tree.js";
import { DevicesManager } from "./devices/manager.js";
import { formatCommand, showLogsCommand } from "./format/commands.js";
import { SwiftFormattingProvider, registerFormatProvider, registerRangeFormatProvider } from "./format/formatter.js";
import { createFormatStatusItem } from "./format/status.js";
import {
  openSimulatorCommand,
  removeSimulatorCacheCommand,
  startSimulatorCommand,
  stopSimulatorCommand,
  takeSimulatorScreenshotCommand,
} from "./simulators/commands.js";
import { SimulatorsManager } from "./simulators/manager.js";
import {
  createIssueGenericCommand,
  createIssueNoSchemesCommand,
  openTerminalPanel,
  resetSweetpadCache,
  testErrorReportingCommand,
} from "./system/commands.js";
import { ProgressStatusBar } from "./system/status-bar.js";
import {
  buildForTestingCommand,
  selectConfigurationForTestingCommand,
  selectTestingTargetCommand,
  selectXcodeSchemeForTestingCommand,
  testWithoutBuildingCommand,
} from "./testing/commands.js";
import { TestingManager } from "./testing/manager.js";
import { installToolCommand, openDocumentationCommand } from "./tools/commands.js";
import { ToolsManager } from "./tools/manager.js";
import { ToolTreeProvider } from "./tools/tree.js";
import { tuistCleanCommand, tuistEditComnmand, tuistGenerateCommand, tuistInstallCommand } from "./tuist/command.js";
import { createTuistWatcher } from "./tuist/watcher.js";
import { xcodgenGenerateCommand } from "./xcodegen/commands.js";
import { createXcodeGenWatcher } from "./xcodegen/watcher.js";
import { createMcpServer } from './mcp_server';
import { McpServerInstance } from './types';


// Keep track of the server instance
let mcpInstance: McpServerInstance | null = null;

export async function activate(context: vscode.ExtensionContext) {
  // Sentry 🚨
  errorReporting.logSetup();

  // 🪵🪓
  Logger.setup();

  try {
    // Check if we have a workspace open
    if (!vscode.workspace.workspaceFolders || vscode.workspace.workspaceFolders.length === 0) {
      // No workspace open, just register minimal commands and exit
      commonLogger.warn("No workspace folder found. Limited functionality available.");
      return;
    }
    
    const buildManager = new BuildManager();
    const devicesManager = new DevicesManager();
    const simulatorsManager = new SimulatorsManager();
    const destinationsManager = new DestinationsManager({
      simulatorsManager: simulatorsManager,
      devicesManager: devicesManager,
    });
    const toolsManager = new ToolsManager();
    const testingManager = new TestingManager();
    const formatter = new SwiftFormattingProvider();
    const progressStatusBar = new ProgressStatusBar();

    // Main context object 🌍
    const _context = new ExtensionContext({
      context: context,
      destinationsManager: destinationsManager,
      buildManager: buildManager,
      toolsManager: toolsManager,
      testingManager: testingManager,
      formatter: formatter,
      progressStatusBar: progressStatusBar,
    });

    // Here is circular dependency, but I don't care
    buildManager.context = _context;
    devicesManager.context = _context;
    destinationsManager.context = _context;
    testingManager.context = _context;
    progressStatusBar.context = _context;

    // --- Perform initial refreshes AFTER context is set ---
    void buildManager.refresh();
    
    // Trees 🎄
    // const buildTreeProvider = new BuildTreeProvider({
    //   context: _context,
    //   buildManager: buildManager,
    // });
    const workspaceTreeProvider = new WorkspaceTreeProvider({
      context: _context,
      buildManager: buildManager,
    });
    const toolsTreeProvider = new ToolTreeProvider({
      manager: toolsManager,
    });
    const destinationsTreeProvider = new DestinationsTreeProvider({
      manager: destinationsManager,
    });

    // Shortcut to push disposable to context.subscriptions
    const d = _context.disposable.bind(_context);
    const command = _context.registerCommand.bind(_context);
    const tree = _context.registerTreeDataProvider.bind(_context);

    const buildTaskProvider = new XcodeBuildTaskProvider(_context);

    // Tasks
    d(vscode.tasks.registerTaskProvider(buildTaskProvider.type, buildTaskProvider));

    // Build
    const schemeStatusBar = new DefaultSchemeStatusBar({
      context: _context,
    });
    d(schemeStatusBar);
    //d(tree("sweetpad.build.view", workspaceTreeProvider));
    d(tree("sweetpad.view.workspaces", workspaceTreeProvider));
    d(command("sweetpad.build.refreshView", async () => buildManager.refresh()));
    d(command("sweetpad.build.launch", launchCommand));
    d(command("sweetpad.build.run", runCommand));
    d(command("sweetpad.build.build", buildCommand));
    d(command("sweetpad.build.clean", cleanCommand));
    d(command("sweetpad.build.test", testCommand));
    d(command("sweetpad.build.testWithSwiftTesting", testWithSwiftTestingCommand));
    d(command("sweetpad.build.resolveDependencies", resolveDependenciesCommand));
    d(command("sweetpad.build.removeBundleDir", removeBundleDirCommand));
    d(command("sweetpad.build.generateBuildServerConfig", generateBuildServerConfigCommand));
    d(command("sweetpad.build.openXcode", openXcodeCommand));
    d(command("sweetpad.build.selectXcodeWorkspace", selectXcodeWorkspaceCommand));
    d(command("sweetpad.build.setDefaultScheme", selectXcodeSchemeForBuildCommand));
    d(command("sweetpad.build.selectConfiguration", selectConfigurationForBuildCommand));
    d(command("sweetpad.build.diagnoseSetup", diagnoseBuildSetupCommand));
    d(command("sweetpad.build.peripheryScan", peripheryScanCommand));
    d(command("sweetpad.build.buildAndPeripheryScan", buildAndPeripheryScanCommand));
    d(command("sweetpad.build.createPeripheryConfig", createPeripheryConfigCommand));

    // Testing
    d(command("sweetpad.testing.buildForTesting", buildForTestingCommand));
    d(command("sweetpad.testing.testWithoutBuilding", testWithoutBuildingCommand));
    d(command("sweetpad.testing.selectTarget", selectTestingTargetCommand));
    d(command("sweetpad.testing.setDefaultScheme", selectXcodeSchemeForTestingCommand));
    d(command("sweetpad.testing.selectConfiguration", selectConfigurationForTestingCommand));

    // Debugging
    d(registerDebugConfigurationProvider(_context));
    d(command("sweetpad.debugger.getAppPath", getAppPathCommand));
    d(command("sweetpad.debugger.debuggingLaunch", debuggingLaunchCommand));
    d(command("sweetpad.debugger.debuggingRun", debuggingRunCommand));
    d(command("sweetpad.debugger.debuggingBuild", debuggingBuildCommand));

    // XcodeGen
    d(command("sweetpad.xcodegen.generate", xcodgenGenerateCommand));
    d(createXcodeGenWatcher(_context));

    // Tuist
    d(command("sweetpad.tuist.generate", tuistGenerateCommand));
    d(command("sweetpad.tuist.install", tuistInstallCommand));
    d(command("sweetpad.tuist.clean", tuistCleanCommand));
    d(command("sweetpad.tuist.edit", tuistEditComnmand));
    d(createTuistWatcher(_context));

    // Format
    d(createFormatStatusItem());
    d(registerFormatProvider(formatter));
    d(registerRangeFormatProvider(formatter));
    d(command("sweetpad.format.run", formatCommand));
    d(command("sweetpad.format.showLogs", showLogsCommand));

    // Simulators
    d(command("sweetpad.simulators.refresh", async () => await destinationsManager.refreshSimulators()));
    d(command("sweetpad.simulators.openSimulator", openSimulatorCommand));
    d(command("sweetpad.simulators.removeCache", removeSimulatorCacheCommand));
    d(command("sweetpad.simulators.start", startSimulatorCommand));
    d(command("sweetpad.simulators.stop", stopSimulatorCommand));
    d(command("sweetpad.simulators.screenshot", takeSimulatorScreenshotCommand));

    // // Devices
    d(command("sweetpad.devices.refresh", async () => await destinationsManager.refreshDevices()));

    // Desintations
    const destinationBar = new DestinationStatusBar({
      context: _context,
    });
    d(destinationBar);
    d(command("sweetpad.destinations.select", selectDestinationForBuildCommand));
    d(command("sweetpad.destinations.removeRecent", removeRecentDestinationCommand));
    d(command("sweetpad.destinations.selectForTesting", selectDestinationForTestingCommand));
    d(tree("sweetpad.destinations.view", destinationsTreeProvider));

    // Tools
    d(tree("sweetpad.tools.view", toolsTreeProvider));
    d(command("sweetpad.tools.install", installToolCommand));
    d(command("sweetpad.tools.refresh", async () => toolsManager.refresh()));
    d(command("sweetpad.tools.documentation", openDocumentationCommand));

    // System
    d(command("sweetpad.system.resetSweetpadCache", resetSweetpadCache));
    d(command("sweetpad.system.createIssue.generic", createIssueGenericCommand));
    d(command("sweetpad.system.createIssue.noSchemes", createIssueNoSchemesCommand));
    d(command("sweetpad.system.testErrorReporting", testErrorReportingCommand));
    d(command("sweetpad.system.openTerminalPanel", openTerminalPanel));

    // --- MCP Server Setup --- 
    commonLogger.log("Starting MCP Server setup...");
    try {
      mcpInstance = createMcpServer({
          name: "SweetpadCommandRunner", 
          version: context.extension.packageJSON.version,
          port: 61337
      }, _context);

      // Start the server
      await mcpInstance.start(); 
      commonLogger.log("MCP Server setup complete and started.");

      // Disposal
      context.subscriptions.push({
        dispose: () => {
          commonLogger.log("Disposing MCP Server subscription...");
          if (mcpInstance?.server) {
               try { mcpInstance.server.close(); } catch(e) { /* log error */ }
          }
          mcpInstance = null;
        }
      });

    } catch (error: unknown) {
      commonLogger.error(`Failed during MCP Server setup`, { error });
      const errorMessage = error instanceof Error ? error.message : String(error);
      vscode.window.showErrorMessage(`Failed to initialize MCP Server: ${errorMessage}`);
    }
  } catch (error: unknown) {
    commonLogger.error("Failed to activate extension", { error });
    const errorMessage = error instanceof Error ? error.message : String(error);
    vscode.window.showErrorMessage(`SweetPad activation failed: ${errorMessage}`);
  }
}

export function deactivate() {
    commonLogger.log("Sweetpad deactivating...");
    // Cleanup is handled by the disposable
}
