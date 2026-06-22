import * as vscode from "vscode";

import { bspDoctorCommand, bspSetupCommand, bspShowLogsCommand } from "./bsp/commands.js";
import { BspService } from "./bsp/service.js";
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
import { BuildManager } from "./build/manager.js";
import { XcodeBuildTaskProvider } from "./build/provider.js";
import { SchemeWatcher } from "./build/scheme-watcher.js";
import { DefaultSchemeStatusBar } from "./build/status-bar.js";
import { BuildTreeProvider } from "./build/tree.js";
import { getWorkspacePath, notifyCustomXcodebuildReadOnlyScope } from "./build/utils.js";
import { CliServerService } from "./cli-server/service.js";
import { type AppDeps, registerCommand, registerTreeDataProvider } from "./common/commands.js";
import { errorReporting } from "./common/error-reporting.js";
import { ExecutionScopeService } from "./common/execution-scope.js";
import { Logger } from "./common/logger.js";
import { warmShellEnv } from "./common/tasks/shell-env.js";
import { WorkspaceStateService } from "./common/workspace-state.js";
import { getAppPathCommand } from "./debugger/commands.js";
import { registerDebugConfigurationProvider } from "./debugger/provider.js";
import {
  removeRecentDestinationCommand,
  searchDestinationsViewCommand,
  selectDestinationForBuildCommand,
  selectDestinationForTestingCommand,
} from "./destination/commands.js";
import { DestinationsManager } from "./destination/manager.js";
import { DestinationStatusBar } from "./destination/status-bar.js";
import { DestinationsTreeProvider } from "./destination/tree.js";
import { DevicesManager } from "./devices/manager.js";
import { TunnelManager } from "./devices/tunnel.js";
import { formatCommand, showLogsCommand } from "./format/commands.js";
import { SwiftFormattingProvider, registerFormatProvider, registerRangeFormatProvider } from "./format/formatter.js";
import { createFormatStatusItem } from "./format/status.js";
import { PreviewsCodeLensProvider } from "./previews/codelens.js";
import {
  refreshPreviewsCommand,
  renderPreviewCommand,
  screenshotPreviewCommand,
  screenshotPreviewVariantsCommand,
  setupPreviewHostCommand,
} from "./previews/commands.js";
import { PreviewHostManager } from "./previews/host.js";
import { PreviewsManager } from "./previews/manager.js";
import { PreviewsTreeProvider } from "./previews/tree.js";
import {
  copySimulatorStreamUrlCommand,
  openSimulatorCommand,
  openSimulatorStreamInBrowserCommand,
  removeSimulatorCacheCommand,
  startSimulatorCommand,
  stopSimulatorCommand,
  streamSimulatorCommand,
} from "./simulators/commands.js";
import { SimulatorsManager } from "./simulators/manager.js";
import { ServeSimManager } from "./simulators/serve-sim.js";
import {
  copyServerNameCommand,
  createIssueGenericCommand,
  createIssueNoSchemesCommand,
  openTerminalPanel,
  refreshShellEnvCommand,
  resetSweetPadCache,
  restartServerCommand,
  showServerStatusCommand,
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
import { installPymobiledevice3Command, installToolCommand, openDocumentationCommand } from "./tools/commands.js";
import { ToolsManager } from "./tools/manager.js";
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

  // SwiftUI Previews and simulator streaming are still in development, so they
  // surface only when the extension runs from source (the Extension Development
  // Host), never in a published install. The context key gates their
  // package.json contributions (the view, command palette, and menus); the
  // guards below skip registering the entrypoints and starting the indexer.
  const devFeaturesEnabled = context.extensionMode === vscode.ExtensionMode.Development;
  await vscode.commands.executeCommand("setContext", "sweetpad.devFeatures", devFeaturesEnabled);

  const workspacePath = getWorkspacePath();

  warmShellEnv();

  // Services 🔧
  // Leaf services with no manager dependencies. Constructed first so managers can take them as deps.
  const workspaceState = new WorkspaceStateService(context);
  const execution = new ExecutionScopeService();

  // Managers 💼
  // These classes are responsible for managing the state of the specific domain. Other parts of the extension can
  // interact with them to get the current state of the domain and subscribe to changes. For example
  // "DestinationsManager" have methods to get the list of current ios devices and simulators, and it also have an
  // event emitter that emits an event when the list of devices or simulators changes.
  const progressStatusBar = new ProgressStatusBar({ execution: execution });
  const tunnelManager = new TunnelManager();
  const devicesManager = new DevicesManager({ vscodeContext: context });
  const simulatorsManager = new SimulatorsManager();
  const destinationsManager = new DestinationsManager({
    simulatorsManager: simulatorsManager,
    devicesManager: devicesManager,
    workspaceState: workspaceState,
  });
  const lspDiagnostics = new LspDiagnosticsService(workspaceState);
  const diagnostics = new DiagnosticsManager();
  const buildManager = new BuildManager({
    workspaceState: workspaceState,
    progress: progressStatusBar,
    execution: execution,
    tunnel: tunnelManager,
    vscodeContext: context,
    destinations: destinationsManager,
    diagnostics: diagnostics,
  });
  const toolsManager = new ToolsManager();
  const serveSimManager = new ServeSimManager();
  const previewsManager = new PreviewsManager();
  const previewHostManager = new PreviewHostManager({
    destinationsManager: destinationsManager,
    serveSimManager: serveSimManager,
    workspaceState: workspaceState,
  });
  const testingManager = new TestingManager({
    workspaceState: workspaceState,
    progress: progressStatusBar,
    execution: execution,
    buildManager: buildManager,
    destinations: destinationsManager,
  });
  const formatter = new SwiftFormattingProvider();

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
  const previewsTreeProvider = new PreviewsTreeProvider({
    manager: previewsManager,
  });

  // Status bars & providers 📊
  const schemeStatusBar = new DefaultSchemeStatusBar({ buildManager: buildManager });
  const destinationBar = new DestinationStatusBar({ destinationsManager: destinationsManager });
  const buildTaskProvider = new XcodeBuildTaskProvider({
    buildManager: buildManager,
    destinationsManager: destinationsManager,
    workspaceState: workspaceState,
    progressStatusBar: progressStatusBar,
    execution: execution,
  });

  // Watchers 👀
  const schemeWatcher = new SchemeWatcher(buildManager);
  const tuistWatcher = new TuistGenWatcher();
  const xcodegenWatcher = new XcodeGenWatcher();
  const serverService = new CliServerService({
    buildManager: buildManager,
    destinationsManager: destinationsManager,
    workspaceState: workspaceState,
    workspacePath: workspacePath,
    extensionVersion: context.extension?.packageJSON?.version ?? "unknown",
    vscodeContext: context,
  });
  const bspService = new BspService({
    buildManager: buildManager,
    workspaceState: workspaceState,
  });

  // One-time (per workspace) note about how a customized xcodebuild command
  // interacts with the bundled resolver — re-checked when the setting changes.
  notifyCustomXcodebuildReadOnlyScope(workspaceState);
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration("sweetpad.build.xcodebuildCommand")) {
        notifyCustomXcodebuildReadOnlyScope(workspaceState);
      }
    }),
  );

  // Start everything that has side effects (subscriptions, calculations, .show(), etc.)
  void progressStatusBar.start();
  void tunnelManager.start();
  void destinationsManager.start();
  void buildManager.start();
  void testingManager.start();
  void buildTreeProvider.start();
  void toolsTreeProvider.start();
  void destinationsTreeProvider.start();
  if (devFeaturesEnabled) {
    previewsManager.start();
    previewsTreeProvider.start();
  }
  void schemeStatusBar.start();
  void destinationBar.start();
  void schemeWatcher.start();
  void tuistWatcher.start();
  void xcodegenWatcher.start();
  void serverService.start();
  void bspService.start();

  // Main dependency bag for commands 🌍
  const deps: AppDeps = {
    destinationsManager: destinationsManager,
    buildManager: buildManager,
    toolsManager: toolsManager,
    testingManager: testingManager,
    formatter: formatter,
    progressStatusBar: progressStatusBar,
    tunnelManager: tunnelManager,
    workspaceState: workspaceState,
    execution: execution,
    vscodeContext: context,
    buildTreeProvider: buildTreeProvider,
    lspDiagnostics: lspDiagnostics,
    serverService: serverService,
    bspService: bspService,
    serveSimManager: serveSimManager,
    previewsManager: previewsManager,
    previewHostManager: previewHostManager,
  };

  // Shortcut helpers bound to the deps bag
  const d = (disposable: vscode.Disposable) => context.subscriptions.push(disposable);
  const command = <Args extends unknown[]>(name: string, cb: (deps: AppDeps, ...args: Args) => Promise<unknown>) =>
    registerCommand(deps, name, cb);
  const tree = registerTreeDataProvider;

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

  // BSP server
  d(command("sweetpad.bsp.setup", bspSetupCommand));
  d(command("sweetpad.bsp.doctor", bspDoctorCommand));
  d(command("sweetpad.bsp.showLogs", bspShowLogsCommand));

  // Testing
  d(command("sweetpad.testing.buildForTesting", buildForTestingCommand));
  d(command("sweetpad.testing.testWithoutBuilding", testWithoutBuildingCommand));
  d(command("sweetpad.testing.selectTarget", selectTestingTargetCommand));
  d(command("sweetpad.testing.setDefaultScheme", selectXcodeSchemeForTestingCommand));
  d(command("sweetpad.testing.selectConfiguration", selectConfigurationForTestingCommand));

  // Debugging
  d(
    registerDebugConfigurationProvider({
      workspaceState: workspaceState,
      vscodeContext: context,
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
  if (devFeaturesEnabled) {
    d(command("sweetpad.simulators.stream", streamSimulatorCommand));
    d(command("sweetpad.simulators.streamOpenInBrowser", openSimulatorStreamInBrowserCommand));
    d(command("sweetpad.simulators.streamCopyUrl", copySimulatorStreamUrlCommand));
    d(serveSimManager);
  }

  // SwiftUI Previews
  if (devFeaturesEnabled) {
    d(tree("sweetpad.previews.view", previewsTreeProvider));
    d(
      vscode.languages.registerCodeLensProvider(
        { language: "swift", scheme: "file" },
        new PreviewsCodeLensProvider({ manager: previewsManager }),
      ),
    );
    d(command("sweetpad.previews.render", renderPreviewCommand));
    d(command("sweetpad.previews.refresh", refreshPreviewsCommand));
    d(command("sweetpad.previews.setup", setupPreviewHostCommand));
    d(command("sweetpad.previews.screenshot", screenshotPreviewCommand));
    d(command("sweetpad.previews.screenshotVariants", screenshotPreviewVariantsCommand));
    d(previewsManager);
  }

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

  // Server
  d(command("sweetpad.cliServer.copyName", copyServerNameCommand));
  d(command("sweetpad.cliServer.restart", restartServerCommand));
  d(command("sweetpad.cliServer.showStatus", showServerStatusCommand));

  lspDiagnostics.reattachIfEnabled();
  lspDiagnostics.showPostReloadNotificationIfPending();
  d(lspDiagnostics);
  d(diagnostics);
  d(serverService);
  d(bspService);
}

export function deactivate() {}
