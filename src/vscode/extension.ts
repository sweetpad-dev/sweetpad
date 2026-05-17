import * as vscode from "vscode";

import { BuildManager } from "../core/build/manager.js";
import { DestinationsManager } from "../core/destination/manager.js";
import { DevicesManager } from "../core/devices/manager.js";
import { ExecutionScopeService } from "../core/execution-scope.js";
import { SimulatorsManager } from "../core/simulators/manager.js";
import { warmShellEnv } from "../core/tasks/shell-env.js";
import { ToolsManager } from "../core/tools/manager.js";
import { VsCodeAsker } from "./adapters/asker.js";
import { VsCodeConfigProvider } from "./adapters/config.js";
import { VsCodeLspRefresher } from "./adapters/lsp.js";
import { VsCodeNotifier } from "./adapters/notifier.js";
import { VsCodeWorkspaceState } from "./adapters/state.js";
import { VsCodeWorkspaceRoot } from "./adapters/workspace-root.js";
import {
  applySchemeFilterCommand,
  buildCommand,
  cleanCommand,
  debuggingBuildCommand,
  debuggingLaunchCommand,
  debuggingRunCommand,
  diagnoseBuildSetupCommand,
  disableLspDiagnosticsCommand,
  enableLspDiagnosticsCommand,
  generateBuildServerConfigCommand,
  launchCommand,
  openXcodeCommand,
  pauseSchemeFilterCommand,
  refreshSchemesCommand,
  removeBundleDirCommand,
  resolveDependenciesCommand,
  runCommand,
  searchBuildViewCommand,
  selectConfigurationForBuildCommand,
  selectXcodeSchemeForBuildCommand,
  selectXcodeWorkspaceCommand,
  stopSchemeCommand,
  switchWorktreeCommand,
  testCommand,
} from "./build/commands.js";
import { DiagnosticsManager } from "./build/diagnostics.js";
import { LspDiagnosticsService } from "./build/lsp-diagnostics.js";
import { XcodeBuildTaskProvider } from "./build/provider.js";
import { SchemeWatcher } from "./build/scheme-watcher.js";
import { DefaultSchemeStatusBar } from "./build/status-bar.js";
import { BuildTreeProvider } from "./build/tree.js";
import { type AppDeps, registerCommand, registerTreeDataProvider } from "./commands.js";
import { getAppPathCommand } from "./debugger/commands.js";
import { registerDebugConfigurationProvider } from "./debugger/provider.js";
import {
  removeRecentDestinationCommand,
  searchDestinationsViewCommand,
  selectDestinationForBuildCommand,
  selectDestinationForTestingCommand,
} from "./destination/commands.js";
import { DestinationStatusBar } from "./destination/status-bar.js";
import { DestinationsTreeProvider } from "./destination/tree.js";
import { TunnelManager } from "./devices/tunnel.js";
import { errorReporting } from "./error-reporting.js";
import { formatCommand, showLogsCommand } from "./format/commands.js";
import { SwiftFormattingProvider, registerFormatProvider, registerRangeFormatProvider } from "./format/formatter.js";
import { createFormatStatusItem } from "./format/status.js";
import { Logger } from "./logger.js";
import { commonLogger } from "./logger.js";
import { ServerClient, extensionOutDirFromContext } from "./server-client.js";
import {
  openSimulatorCommand,
  removeSimulatorCacheCommand,
  startSimulatorCommand,
  stopSimulatorCommand,
} from "./simulators/commands.js";
import {
  createIssueGenericCommand,
  createIssueNoSchemesCommand,
  openTerminalPanel,
  refreshShellEnvCommand,
  resetSweetPadCache,
  testErrorReportingCommand,
} from "./system/commands.js";
import { ProgressStatusBar } from "./system/status-bar.js";
import { VsCodeTaskRunner } from "./tasks/run.js";
import {
  buildForTestingCommand,
  selectConfigurationForTestingCommand,
  selectTestingTargetCommand,
  selectXcodeSchemeForTestingCommand,
  testWithoutBuildingCommand,
} from "./testing/commands.js";
import { TestingManager } from "./testing/manager.js";
import { installPymobiledevice3Command, installToolCommand, openDocumentationCommand } from "./tools/commands.js";
import { ToolTreeProvider } from "./tools/tree.js";
import {
  tuistCleanCommand,
  tuistEditComnmand,
  tuistGenerateCommand,
  tuistInstallCommand,
  tuistTestComnmand,
} from "./tuist/command.js";
import { TuistGenWatcher } from "./tuist/watcher.js";
import { xcodgenGenerateCommand } from "./xcodegen/commands.js";
import { XcodeGenWatcher } from "./xcodegen/watcher.js";

export async function activate(context: vscode.ExtensionContext) {
  // Sentry 🚨
  errorReporting.logSetup();

  // 🪵🪓
  Logger.setup();

  // An activation event matched this workspace — reveal SweetPad UI.
  await vscode.commands.executeCommand("setContext", "sweetpad.enabled", true);

  const config = new VsCodeConfigProvider();
  const asker = new VsCodeAsker();
  const notifier = new VsCodeNotifier();
  const workspaceRoot = new VsCodeWorkspaceRoot(context);
  const lspRefresher = new VsCodeLspRefresher(config, commonLogger);

  // Best-effort shell env warm-up. Skipped quietly if no workspace folder is open yet.
  const cwd = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  warmShellEnv({
    logger: commonLogger,
    shell: config.get("shellEnv.shell"),
    timeoutMs: config.get("shellEnv.timeout"),
    cwd: cwd,
    onWarning: (message) => notifier.warn(message),
  });

  // Services 🔧
  // Leaf services with no manager dependencies. Constructed first so managers can take them as deps.
  const workspace = new VsCodeWorkspaceState(context);
  const execution = new ExecutionScopeService();
  const taskRunner = new VsCodeTaskRunner({
    execution: execution,
    config: config,
    workspaceRoot: workspaceRoot,
    logger: commonLogger,
  });

  // Note: managers carry `workspaceRoot` rather than pre-resolved `cwd`/`storagePath`
  // strings, so activation succeeds on a swift-file-only window (no folder open).
  // The "No workspace folder found" error only surfaces when the user actually
  // runs a build command.

  // Managers 💼
  const progressStatusBar = new ProgressStatusBar({ execution: execution });
  const tunnelManager = new TunnelManager({ workspaceRoot: workspaceRoot, config: config, logger: commonLogger });
  const devicesManager = new DevicesManager({
    logger: commonLogger,
    workspaceRoot: workspaceRoot,
  });
  const simulatorsManager = new SimulatorsManager({
    logger: commonLogger,
    config: config,
    workspaceRoot: workspaceRoot,
  });
  const destinationsManager = new DestinationsManager({
    simulatorsManager: simulatorsManager,
    devicesManager: devicesManager,
    workspace: workspace,
  });
  const lspDiagnostics = new LspDiagnosticsService(workspace);
  const diagnostics = new DiagnosticsManager();
  const buildManager = new BuildManager({
    logger: commonLogger,
    config: config,
    state: workspace,
    asker: asker,
    progress: progressStatusBar,
    taskRunner: taskRunner,
    notifier: notifier,
    lsp: lspRefresher,
    destinations: destinationsManager,
    diagnostics: diagnostics,
    workspaceRoot: workspaceRoot,
    beforeDeviceLaunch: () => tunnelManager.autoConnect(),
  });
  const toolsManager = new ToolsManager({ logger: commonLogger, workspaceRoot: workspaceRoot });
  const testingManager = new TestingManager({
    workspace: workspace,
    progress: progressStatusBar,
    execution: execution,
    buildManager: buildManager,
    destinations: destinationsManager,
    asker: asker,
    workspaceRoot: workspaceRoot,
    config: config,
    logger: commonLogger,
    taskRunner: taskRunner,
  });
  const formatter = new SwiftFormattingProvider({
    workspaceRoot: workspaceRoot,
    config: config,
    logger: commonLogger,
  });

  // Trees 🎄
  const buildTreeProvider = new BuildTreeProvider({
    buildManager: buildManager,
  });
  const toolsTreeProvider = new ToolTreeProvider({
    manager: toolsManager,
  });
  const destinationsTreeProvider = new DestinationsTreeProvider({
    manager: destinationsManager,
  });

  // Status bars & providers 📊
  const schemeStatusBar = new DefaultSchemeStatusBar({ buildManager: buildManager });
  const destinationBar = new DestinationStatusBar({ destinationsManager: destinationsManager });
  // Task provider needs the full AppDeps after the engine refactor; construct it
  // lazily after `deps` is built below.
  let buildTaskProvider: XcodeBuildTaskProvider;

  // Watchers 👀
  const schemeWatcher = new SchemeWatcher({
    buildManager: buildManager,
    workspaceRoot: workspaceRoot,
    config: config,
    logger: commonLogger,
  });
  const tuistWatcher = new TuistGenWatcher({ workspaceRoot: workspaceRoot, config: config, logger: commonLogger });
  const xcodegenWatcher = new XcodeGenWatcher({ workspaceRoot: workspaceRoot, config: config, logger: commonLogger });

  // Start everything that has side effects (subscriptions, calculations, .show(), etc.)
  void progressStatusBar.start();
  void tunnelManager.start();
  void destinationsManager.start();
  void buildManager.start();
  void testingManager.start();
  void buildTreeProvider.start();
  void toolsTreeProvider.start();
  void destinationsTreeProvider.start();
  void schemeStatusBar.start();
  void destinationBar.start();
  void schemeWatcher.start();
  void tuistWatcher.start();
  void xcodegenWatcher.start();

  // Phase 3: standalone-server client. Always constructed so the
  // `AppDeps` bag is uniform; `buildCommand` checks the
  // `system.experimental.serverMode` flag at dispatch time to decide
  // whether to use it.
  const serverClient = new ServerClient({
    logger: commonLogger,
    workspaceRoot: workspaceRoot,
    config: config,
    extensionOutDir: extensionOutDirFromContext(context),
  });

  // Main dependency bag for commands 🌍
  const deps: AppDeps = {
    destinationsManager: destinationsManager,
    buildManager: buildManager,
    devicesManager: devicesManager,
    toolsManager: toolsManager,
    testingManager: testingManager,
    formatter: formatter,
    progressStatusBar: progressStatusBar,
    tunnelManager: tunnelManager,
    workspace: workspace,
    execution: execution,
    vscodeContext: context,
    buildTreeProvider: buildTreeProvider,
    lspDiagnostics: lspDiagnostics,
    asker: asker,
    logger: commonLogger,
    config: config,
    lspRefresher: lspRefresher,
    taskRunner: taskRunner,
    workspaceRoot: workspaceRoot,
    serverClient: serverClient,
    diagnostics: diagnostics,
  };
  buildTaskProvider = new XcodeBuildTaskProvider(deps);

  // Shortcut helpers bound to the deps bag
  const d = (disposable: vscode.Disposable) => context.subscriptions.push(disposable);
  const command = <Args extends unknown[]>(name: string, cb: (deps: AppDeps, ...args: Args) => Promise<unknown>) =>
    registerCommand(deps, name, cb);
  const tree = registerTreeDataProvider;

  d(serverClient);

  // Tasks
  d(vscode.tasks.registerTaskProvider(buildTaskProvider.type, buildTaskProvider));

  // Build
  d(schemeStatusBar);
  d(tree("sweetpad.build.view", buildTreeProvider));
  d(command("sweetpad.build.refreshSchemes", refreshSchemesCommand));
  d(command("sweetpad.build.launch", launchCommand));
  d(command("sweetpad.build.run", runCommand));
  d(command("sweetpad.build.build", buildCommand));
  d(command("sweetpad.build.clean", cleanCommand));
  d(command("sweetpad.build.test", testCommand));
  d(command("sweetpad.build.resolveDependencies", resolveDependenciesCommand));
  d(command("sweetpad.build.removeBundleDir", removeBundleDirCommand));
  d(command("sweetpad.build.generateBuildServerConfig", generateBuildServerConfigCommand));
  d(command("sweetpad.build.enableLspDiagnostics", enableLspDiagnosticsCommand));
  d(command("sweetpad.build.disableLspDiagnostics", disableLspDiagnosticsCommand));
  d(command("sweetpad.build.openXcode", openXcodeCommand));
  d(command("sweetpad.build.selectXcodeWorkspace", selectXcodeWorkspaceCommand));
  d(command("sweetpad.build.setDefaultScheme", selectXcodeSchemeForBuildCommand));
  d(command("sweetpad.build.selectConfiguration", selectConfigurationForBuildCommand));
  d(command("sweetpad.build.diagnoseSetup", diagnoseBuildSetupCommand));
  d(command("sweetpad.build.stop", stopSchemeCommand));
  d(command("sweetpad.build.switchWorktree", switchWorktreeCommand));
  d(command("sweetpad.build.pauseSchemeFilter", pauseSchemeFilterCommand));
  d(command("sweetpad.build.applySchemeFilter", applySchemeFilterCommand));
  d(command("sweetpad.build.search", searchBuildViewCommand));

  // Testing
  d(command("sweetpad.testing.buildForTesting", buildForTestingCommand));
  d(command("sweetpad.testing.testWithoutBuilding", testWithoutBuildingCommand));
  d(command("sweetpad.testing.selectTarget", selectTestingTargetCommand));
  d(command("sweetpad.testing.setDefaultScheme", selectXcodeSchemeForTestingCommand));
  d(command("sweetpad.testing.selectConfiguration", selectConfigurationForTestingCommand));

  // Debugging
  d(
    registerDebugConfigurationProvider({
      workspace: workspace,
      vscodeContext: context,
      workspaceRoot: workspaceRoot,
      logger: commonLogger,
    }),
  );
  d(command("sweetpad.debugger.getAppPath", getAppPathCommand));
  d(command("sweetpad.debugger.debuggingLaunch", debuggingLaunchCommand));
  d(command("sweetpad.debugger.debuggingRun", debuggingRunCommand));
  d(command("sweetpad.debugger.debuggingBuild", debuggingBuildCommand));

  // XcodeGen
  d(command("sweetpad.xcodegen.generate", xcodgenGenerateCommand));
  d(xcodegenWatcher);

  // Tuist
  d(command("sweetpad.tuist.generate", tuistGenerateCommand));
  d(command("sweetpad.tuist.install", tuistInstallCommand));
  d(command("sweetpad.tuist.clean", tuistCleanCommand));
  d(command("sweetpad.tuist.edit", tuistEditComnmand));
  d(command("sweetpad.tuist.test", tuistTestComnmand));
  d(tuistWatcher);

  // Scheme Auto-Refresh Watcher
  d(schemeWatcher);

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

  // // Devices
  d(command("sweetpad.devices.refresh", async () => await destinationsManager.refreshDevices()));
  d(tunnelManager);

  // Desintations
  d(destinationBar);
  d(command("sweetpad.destinations.select", selectDestinationForBuildCommand));
  d(command("sweetpad.destinations.removeRecent", removeRecentDestinationCommand));
  d(command("sweetpad.destinations.selectForTesting", selectDestinationForTestingCommand));
  d(command("sweetpad.destinations.search", searchDestinationsViewCommand));
  d(tree("sweetpad.destinations.view", destinationsTreeProvider));

  // Tools
  d(tree("sweetpad.tools.view", toolsTreeProvider));
  d(command("sweetpad.tools.install", installToolCommand));
  d(command("sweetpad.tools.installPymobiledevice3", installPymobiledevice3Command));
  d(command("sweetpad.tools.refresh", async () => toolsManager.refresh()));
  d(command("sweetpad.tools.documentation", openDocumentationCommand));

  // System
  d(command("sweetpad.system.resetSweetPadCache", resetSweetPadCache));
  d(command("sweetpad.system.createIssue.generic", createIssueGenericCommand));
  d(command("sweetpad.system.createIssue.noSchemes", createIssueNoSchemesCommand));
  d(command("sweetpad.system.testErrorReporting", testErrorReportingCommand));
  d(command("sweetpad.system.openTerminalPanel", openTerminalPanel));
  d(command("sweetpad.system.refreshShellEnv", refreshShellEnvCommand));

  lspDiagnostics.reattachIfEnabled();
  lspDiagnostics.showPostReloadNotificationIfPending();
  d(lspDiagnostics);
  d(diagnostics);
}

export function deactivate() {}
