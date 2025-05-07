import * as vscode from "vscode";
import {
  buildCommand,
  cleanCommand,
  debuggingBuildCommand,
  debuggingLaunchCommand,
  debuggingRunCommand,
  diagnoseBuildSetupCommand,
  generateBuildServerConfigCommand,
  launchCommand,
  openXcodeCommand,
  removeBundleDirCommand,
  resolveDependenciesCommand,
  runCommand,
  selectConfigurationForBuildCommand,
  selectXcodeSchemeForBuildCommand,
  selectXcodeWorkspaceCommand,
  testCommand,
} from "./build/commands.js";
import { BuildManager } from "./build/manager.js";
import { XcodeBuildTaskProvider } from "./build/provider.js";
import { DefaultSchemeStatusBar } from "./build/status-bar.js";
import { BuildTreeProvider } from "./build/tree.js";
import { ExtensionContext } from "./common/commands.js";
import { errorReporting } from "./common/error-reporting.js";
import { Logger } from "./common/logger.js";
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
import { SwiftFormattingProvider, registerFormatProvider } from "./format/formatter.js";
import { createFormatStatusItem } from "./format/status.js";
import {
  openSimulatorCommand,
  removeSimulatorCacheCommand,
  startSimulatorCommand,
  stopSimulatorCommand,
} from "./simulators/commands.js";
import { SimulatorsManager } from "./simulators/manager.js";
import {
  createIssueGenericCommand,
  createIssueNoSchemesCommand,
  resetSweetpadCache,
  testErrorReportingCommand,
} from "./system/commands.js";
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

export function activate(context: vscode.ExtensionContext) {
  // Sentry 🚨
  errorReporting.logSetup();

  // 🪵🪓
  Logger.setup();

  // Managers 💼
  // These classes are responsible for managing the state of the specific domain. Other parts of the extension can
  // interact with them to get the current state of the domain and subscribe to changes. For example
  // "DestinationsManager" have methods to get the list of current ios devices and simulators, and it also have an
  // event emitter that emits an event when the list of devices or simulators changes.
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

  // Main context object 🌍
  const _context = new ExtensionContext({
    context: context,
    destinationsManager: destinationsManager,
    buildManager: buildManager,
    toolsManager: toolsManager,
    testingManager: testingManager,
    formatter: formatter,
  });
  // Here is circular dependency, but I don't care
  buildManager.context = _context;
  devicesManager.context = _context;
  destinationsManager.context = _context;
  testingManager.context = _context;

  // Trees 🎄
  const buildTreeProvider = new BuildTreeProvider({
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
  d(tree("sweetpad.build.view", buildTreeProvider));
  d(command("sweetpad.build.refreshView", "Refresh View", async () => buildManager.refresh()));
  d(command("sweetpad.build.launch", "Build & Run", launchCommand));
  d(command("sweetpad.build.run", "Run", runCommand));
  d(command("sweetpad.build.build", "Build", buildCommand));
  d(command("sweetpad.build.clean", "Clean", cleanCommand));
  d(command("sweetpad.build.test", "Test", testCommand));
  d(command("sweetpad.build.resolveDependencies", "Resolve Dependencies", resolveDependenciesCommand));
  d(command("sweetpad.build.removeBundleDir", "Remove Bundle Dir", removeBundleDirCommand));
  d(command("sweetpad.build.generateBuildServerConfig", "Generate buildServer.json", generateBuildServerConfigCommand));
  d(command("sweetpad.build.openXcode", "Open Xcode", openXcodeCommand));
  d(command("sweetpad.build.selectXcodeWorkspace", "Select Xcode Workspace", selectXcodeWorkspaceCommand));
  d(command("sweetpad.build.setDefaultScheme", "Set Default Scheme", selectXcodeSchemeForBuildCommand));
  d(command("sweetpad.build.selectConfiguration", "Select Configuration", selectConfigurationForBuildCommand));
  d(command("sweetpad.build.diagnoseSetup", "Diagnose Setup", diagnoseBuildSetupCommand));

  // Testing
  d(command("sweetpad.testing.buildForTesting", "Build for Testing", buildForTestingCommand));
  d(command("sweetpad.testing.testWithoutBuilding", "Test without Building", testWithoutBuildingCommand));
  d(command("sweetpad.testing.selectTarget", "Select Target", selectTestingTargetCommand));
  d(command("sweetpad.testing.setDefaultScheme", "Set Default Scheme", selectXcodeSchemeForTestingCommand));
  d(command("sweetpad.testing.selectConfiguration", "Select Configuration", selectConfigurationForTestingCommand));

  // Debugging
  d(registerDebugConfigurationProvider(_context));
  d(command("sweetpad.debugger.getAppPath", "Get App Path", getAppPathCommand));
  d(command("sweetpad.debugger.debuggingLaunch", "Debug", debuggingLaunchCommand));
  d(command("sweetpad.debugger.debuggingRun", "Debug (Run only)", debuggingRunCommand));
  d(command("sweetpad.debugger.debuggingBuild", "Debug (Build only)", debuggingBuildCommand));

  // XcodeGen
  d(command("sweetpad.xcodegen.generate", "Generate project using XcodeGen", xcodgenGenerateCommand));
  d(createXcodeGenWatcher(_context));

  // Tuist
  d(command("sweetpad.tuist.generate", "Generate project using Tuist", tuistGenerateCommand));
  d(command("sweetpad.tuist.install", "Install Swift Package using Tuist", tuistInstallCommand));
  d(command("sweetpad.tuist.clean", "Clean Tuist project", tuistCleanCommand));
  d(command("sweetpad.tuist.edit", "Edit Tuist project", tuistEditComnmand));
  d(createTuistWatcher(_context));

  // Format
  d(createFormatStatusItem());
  d(registerFormatProvider(formatter));
  d(command("sweetpad.format.run", "Format", formatCommand));
  d(command("sweetpad.format.showLogs", "Show Logs", showLogsCommand));

  // Simulators
  d(command("sweetpad.simulators.refresh", "Refresh Simulators", async () => await destinationsManager.refreshSimulators()));
  d(command("sweetpad.simulators.openSimulator", "Open Simulator", openSimulatorCommand));
  d(command("sweetpad.simulators.removeCache", "Remove Simulator Cache", removeSimulatorCacheCommand));
  d(command("sweetpad.simulators.start", "Start Simulator", startSimulatorCommand));
  d(command("sweetpad.simulators.stop", "Stop Simulator", stopSimulatorCommand));

  // // Devices
  d(command("sweetpad.devices.refresh", "Refresh Devices", async () => await destinationsManager.refreshDevices()));

  // Desintations
  const destinationBar = new DestinationStatusBar({
    context: _context,
  });
  d(destinationBar);
  d(command("sweetpad.destinations.select", "Select Destination", selectDestinationForBuildCommand));
  d(command("sweetpad.destinations.removeRecent", "Remove Recent Destination", removeRecentDestinationCommand));
  d(command("sweetpad.destinations.selectForTesting", "Select Destination for Testing", selectDestinationForTestingCommand));
  d(tree("sweetpad.destinations.view", destinationsTreeProvider));

  // Tools
  d(tree("sweetpad.tools.view", toolsTreeProvider));
  d(command("sweetpad.tools.install", "Install Tool", installToolCommand));
  d(command("sweetpad.tools.refresh", "Refresh", async () => toolsManager.refresh()));
  d(command("sweetpad.tools.documentation", "Open Tool Documentation", openDocumentationCommand));

  // System
  d(command("sweetpad.system.resetSweetpadCache", "Reset Sweetpad Cache", resetSweetpadCache));
  d(command("sweetpad.system.createIssue.generic", "Create Issue", createIssueGenericCommand));
  d(command("sweetpad.system.createIssue.noSchemes", "Create Issue (No Schemes)", createIssueNoSchemesCommand));
  d(command("sweetpad.system.testErrorReporting", "Test Error Reporting", testErrorReportingCommand));
}

export function deactivate() {}
