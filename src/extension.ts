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
  d(command("sweetpad.build.refreshView", async () => buildManager.refresh()));
  d(command("sweetpad.build.launch", launchCommand));
  d(command("sweetpad.build.run", runCommand));
  d(command("sweetpad.build.build", buildCommand));
  d(command("sweetpad.build.clean", cleanCommand));
  d(command("sweetpad.build.test", testCommand));
  d(command("sweetpad.build.resolveDependencies", resolveDependenciesCommand));
  d(command("sweetpad.build.removeBundleDir", removeBundleDirCommand));
  d(command("sweetpad.build.generateBuildServerConfig", generateBuildServerConfigCommand));
  d(command("sweetpad.build.openXcode", openXcodeCommand));
  d(command("sweetpad.build.selectXcodeWorkspace", selectXcodeWorkspaceCommand));
  d(command("sweetpad.build.setDefaultScheme", selectXcodeSchemeForBuildCommand));
  d(command("sweetpad.build.selectConfiguration", selectConfigurationForBuildCommand));
  d(command("sweetpad.build.diagnoseSetup", diagnoseBuildSetupCommand));

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
  d(command("sweetpad.format.run", formatCommand));
  d(command("sweetpad.format.showLogs", showLogsCommand));

  // Simulators
  d(command("sweetpad.simulators.refresh", async () => await destinationsManager.refreshSimulators()));
  d(command("sweetpad.simulators.openSimulator", openSimulatorCommand));
  d(command("sweetpad.simulators.removeCache", removeSimulatorCacheCommand));
  d(command("sweetpad.simulators.start", startSimulatorCommand));
  d(command("sweetpad.simulators.stop", stopSimulatorCommand));

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
}

export function deactivate() {}
