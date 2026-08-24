import events from "node:events";
import * as path from "node:path";

import * as vscode from "vscode";

import { getBuildServerProvider } from "../bsp/commands";
import {
  type XcodeScheme,
  getBuildSettingsToLaunch,
  getIsXcbeautifyInstalled,
  getIsXBSInstalled,
  getSchemes,
  getSwiftCommand,
  getXcodeBuildCommand,
  getXcodeVersionInstalled,
} from "../common/cli/scripts";
import { getWorkspaceConfig, onDidChangeConfiguration, updateWorkspaceConfig } from "../common/config";
import { ExtensionError } from "../common/errors";
import { BaseExecutionScope, type ExecutionScopeService } from "../common/execution-scope";
import { isFileExists, readJsonFile, tempFilePath } from "../common/files";
import { commonLogger } from "../common/logger";
import { runTask } from "../common/tasks/run";
import type { Command, TaskTerminal } from "../common/tasks/types";
import { assertUnreachable } from "../common/types";
import type { WorkspaceContextService } from "../common/workspace-context";
import type { WorkspaceStateService } from "../common/workspace-state";
import * as iosDeploy from "../common/xcode/ios-deploy";
import type { DestinationsManager } from "../destination/manager";
import type { DestinationType } from "../destination/types";
import type { TunnelManager } from "../devices/tunnel";
import type { DeviceDestination } from "../devices/types";
import { resolveDeviceRunMethod } from "../devices/utils";
import { MainExecutable } from "../run/main";
import { MacOSLogSidecar, Pymd3Sidecar, SimulatorLogSidecar } from "../run/sidecars";
import type { SimulatorDestination } from "../simulators/types";
import { getSimulatorByUdid } from "../simulators/utils";
import type { ProgressStatusBar } from "../system/status-bar";
import { BUILD_TASK_PROBLEM_MATCHERS } from "./constants";
import type { DiagnosticsManager } from "./diagnostics";
import type { ParsedDiagnostic } from "./diagnostics-parser";
import {
  ensureInjectionAppRunning,
  isHotReloadEnabled,
  sdkSupportsHotReload,
  withHotReloadLaunchEnv,
} from "./hot-reload";
import type { BuildTreeItem } from "./tree";
import {
  XcodeCommandBuilder,
  askConfiguration,
  askDestinationToRunOn,
  askSchemeForBuild,
  askXcodeWorkspacePath,
  buildDestinationString,
  detectWorkspaceType,
  ensureAppPathExists,
  generateBuildServerConfigOnBuild,
  activateCurrentXcodeWorkspacePath,
  notifyXBSMissing,
  getSchemeLaunchSettings,
  getSwiftPMDirectory,
  getXcodeBuildDestinationString,
  isAutoGenerateBuildServerConfigEnabled,
  isXcbeautifyEnabled,
  prepareBundleDir,
  prepareDerivedDataPath,
  refreshBuildServer,
  restartSwiftLSP,
  writeWatchMarkers,
  getWorkspaceRoot,
} from "./utils";

// Stable category strings — exposed to CLI consumers, so keep the union narrow.
export type BuildSessionCommand = "build" | "run" | "launch" | "test" | "clean" | "resolve-deps";

export type BuildSessionStarted = {
  scheme: string;
  command: BuildSessionCommand;
};

export type BuildSessionEnded = {
  scheme: string;
  status: "succeeded" | "failed" | "cancelled";
};

type IEventMap = {
  refreshSchemesStarted: [];
  refreshSchemesCompleted: [XcodeScheme[]];
  refreshSchemesFailed: [];

  defaultSchemeForBuildUpdated: [scheme: string | undefined];
  defaultSchemeForTestingUpdated: [scheme: string | undefined];

  defaultConfigurationForBuildUpdated: [configuration: string | undefined];

  schemeBuildStarted: [scheme: string];
  schemeBuildStopped: [scheme: string];

  // Emitted alongside schemeBuildStarted/Stopped but carry richer info —
  // used by the RPC server's BuildSessionRegistry to build the persisted
  // BuildEntity. Kept as separate events so the legacy schemeBuild* signature
  // doesn't have to change.
  buildSessionStarted: [info: BuildSessionStarted];
  buildLogLine: [info: { line: string; diagnostic: ParsedDiagnostic | null }];
  buildSessionEnded: [info: BuildSessionEnded];
};
type IEventKey = keyof IEventMap;

export class BuildManager {
  private cache: XcodeScheme[] | undefined = undefined;
  private emitter = new events.EventEmitter<IEventMap>();
  private workspaceState: WorkspaceStateService;
  private progress: ProgressStatusBar;
  private execution: ExecutionScopeService;
  private tunnel: TunnelManager;
  private vscodeContext: vscode.ExtensionContext;
  private destinations: DestinationsManager;
  private diagnostics: DiagnosticsManager;
  private runningSchemes: Set<string> = new Set();
  private cancellingSchemes: Set<string> = new Set();

  private readonly workspaceContext: WorkspaceContextService;

  constructor(options: {
    workspaceContext: WorkspaceContextService;
    workspaceState: WorkspaceStateService;
    progress: ProgressStatusBar;
    execution: ExecutionScopeService;
    tunnel: TunnelManager;
    vscodeContext: vscode.ExtensionContext;
    destinations: DestinationsManager;
    diagnostics: DiagnosticsManager;
  }) {
    this.workspaceContext = options.workspaceContext;
    this.workspaceState = options.workspaceState;
    this.progress = options.progress;
    this.execution = options.execution;
    this.tunnel = options.tunnel;
    this.vscodeContext = options.vscodeContext;
    this.destinations = options.destinations;
    this.diagnostics = options.diagnostics;
  }

  async start(): Promise<void> {
    this.on("defaultSchemeForBuildUpdated", () => {
      void this.generateXBSSettingsOnSchemeChange({
        scheme: this.getDefaultSchemeForBuild(),
      });
    });

    // A pinned scheme outranks the cache, so editing the setting is a scheme change
    // like any other. Views listen for this event rather than the setting itself.
    onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration("sweetpad.build.scheme")) {
        this.emitter.emit("defaultSchemeForBuildUpdated", this.getDefaultSchemeForBuild());
      }
    });
  }

  on<K extends IEventKey>(event: K, listener: (...args: IEventMap[K]) => void): void {
    this.emitter.on(event, listener as any); // todo: fix this any
  }

  off<K extends IEventKey>(event: K, listener: (...args: IEventMap[K]) => void): void {
    this.emitter.off(event, listener as any);
  }

  removeAllListeners<K extends IEventKey>(event: K): void {
    this.emitter.removeAllListeners(event);
  }

  startSchemeBuild(scheme: string): void {
    this.runningSchemes.add(scheme);
    this.emitter.emit("schemeBuildStarted", scheme);
  }

  stopSchemeBuild(scheme: string): void {
    this.runningSchemes.delete(scheme);
    this.emitter.emit("schemeBuildStopped", scheme);
  }

  isSchemeRunning(scheme: string): boolean {
    return this.runningSchemes.has(scheme);
  }

  async refreshSchemes(): Promise<XcodeScheme[]> {
    const scope = new BaseExecutionScope();
    return await this.execution.startScope(scope, async () => {
      this.progress.updateText("Refreshing Xcode schemes");

      this.emitter.emit("refreshSchemesStarted");
      try {
        const xcworkspace = activateCurrentXcodeWorkspacePath({
          workspaceState: this.workspaceState,
          workspaceContext: this.workspaceContext,
        });

        const schemes = await getSchemes({ xcworkspace: xcworkspace });

        this.cache = schemes;

        await this.validateDefaultSchemes();
        this.emitter.emit("refreshSchemesCompleted", schemes);
        return this.cache;
      } catch (error: unknown) {
        commonLogger.error("Failed to refresh schemes", { error: error });
        this.emitter.emit("refreshSchemesFailed");
        throw error;
      }
    });
  }

  async getSchemes(options?: { refresh?: boolean }): Promise<XcodeScheme[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refreshSchemes();
    }
    return this.cache;
  }

  getDefaultSchemeForBuild(): string | undefined {
    const fromConfig = getWorkspaceConfig("build.scheme");
    if (fromConfig) {
      return fromConfig;
    }
    return this.workspaceState.get("build.xcodeScheme");
  }

  getDefaultSchemeForTesting(): string | undefined {
    return this.workspaceState.get("testing.xcodeScheme");
  }

  setDefaultSchemeForBuild(scheme: string | undefined): void {
    this.workspaceState.update("build.xcodeScheme", scheme);
    this.emitter.emit("defaultSchemeForBuildUpdated", scheme);
  }

  /**
   * Record a scheme pick where it will actually be read. 'sweetpad.build.scheme'
   * outranks the workspace-state cache, so caching a pick while that setting is
   * pinned would report success and change nothing. Pass 'pin' to move a cached
   * pick into settings.
   */
  async persistSchemeForBuild(scheme: string, options?: { pin?: boolean }): Promise<void> {
    if (!options?.pin && !getWorkspaceConfig("build.scheme")) {
      this.setDefaultSchemeForBuild(scheme);
      return;
    }

    await updateWorkspaceConfig("build.scheme", scheme);
    // The setting answers for the scheme now, so drop the cached copy quietly —
    // the configuration change already announced the new value.
    this.workspaceState.update("build.xcodeScheme", undefined);
  }

  setDefaultSchemeForTesting(scheme: string | undefined): void {
    this.workspaceState.update("testing.xcodeScheme", scheme);
    this.emitter.emit("defaultSchemeForTestingUpdated", scheme);
  }

  getDefaultConfigurationForBuild(): string | undefined {
    return this.workspaceState.get("build.xcodeConfiguration");
  }

  getDefaultConfigurationForTesting(): string | undefined {
    return this.workspaceState.get("testing.xcodeConfiguration");
  }

  setDefaultConfigurationForBuild(configuration: string | undefined): void {
    this.workspaceState.update("build.xcodeConfiguration", configuration);
    this.emitter.emit("defaultConfigurationForBuildUpdated", configuration);
  }

  setDefaultConfigurationForTesting(configuration: string | undefined): void {
    this.workspaceState.update("testing.xcodeConfiguration", configuration);
  }

  /**
   * Every time the scheme changes, we need to rebuild the buildServer.json file
   * for providing the correct build settings to the LSP server.
   */
  async generateXBSSettingsOnSchemeChange(options: { scheme: string | undefined }): Promise<void> {
    if (!options.scheme) {
      return;
    }

    if (!isAutoGenerateBuildServerConfigEnabled()) {
      return;
    }

    // xcode-build-server bakes the scheme into buildServer.json, so a scheme
    // change invalidates it. SweetPad's own config names no scheme — the scheme
    // lives in `bsp.json`, which `BspService` rewrites from the same event — so
    // there is nothing to regenerate here, and asking for a tool that provider
    // never uses would be a prompt to install xcode-build-server for nothing.
    if (getBuildServerProvider() === "sweetpad") {
      return;
    }

    // The precondition is about the folder SweetPad is on right now — read once, before the
    // picker below can move it. The refresh further down targets the folder of the project the
    // picker actually returned, which is a different question.
    const activeRoot = this.workspaceContext.root;
    const buildServerJsonPath = path.join(activeRoot, "buildServer.json");
    const isBuildServerJsonExists = await isFileExists(buildServerJsonPath);
    if (!isBuildServerJsonExists) {
      return;
    }

    const isServerInstalled = await getIsXBSInstalled();
    if (!isServerInstalled) {
      await notifyXBSMissing(this.workspaceState);
      return;
    }

    const xcworkspace = await askXcodeWorkspacePath({
      workspaceState: this.workspaceState,
      workspaceContext: this.workspaceContext,
      buildManager: this,
    });
    const workspaceRoot = getWorkspaceRoot({
      xcworkspace: xcworkspace,
      workspaceContext: this.workspaceContext,
    });

    await refreshBuildServer({
      workspaceRoot: workspaceRoot,
      xcworkspace: xcworkspace,
      scheme: options.scheme,
      configuration: this.getDefaultConfigurationForBuild(),
    });

    const isShown = this.workspaceState.get("build.xbsAutogenreateInfoShown") ?? false;
    if (!isShown) {
      this.workspaceState.update("build.xbsAutogenreateInfoShown", true);
      vscode.window.showInformationMessage(`
          INFO: "buildServer.json" file is automatically regenerated every time you change the scheme.
          If you want to disable this feature, you can do it in the settings. This message is shown only once.
      `);
    }
  }

  /**
   * Validates that the current default schemes still exist in the refreshed schemes list.
   * If a default scheme no longer exists, it will be cleared.
   */
  private async validateDefaultSchemes(): Promise<void> {
    if (!this.cache) {
      return;
    }

    const schemeNames = new Set(this.cache.map((scheme) => scheme.name));
    // Only clear workspace-state caches — settings are intentional and win over cache.
    const cachedBuildScheme = this.workspaceState.get("build.xcodeScheme");
    if (cachedBuildScheme && !schemeNames.has(cachedBuildScheme)) {
      this.setDefaultSchemeForBuild(undefined);
    }

    const currentTestingScheme = this.getDefaultSchemeForTesting();
    if (currentTestingScheme && !schemeNames.has(currentTestingScheme)) {
      this.setDefaultSchemeForTesting(undefined);
    }
  }

  // Wraps runTask with common options for every scheme task (build/run/test/...)
  // and emits the buildSession* events the in-extension RPC server records.
  async runSchemeTask(options: {
    name: string;
    scheme: string;
    workspaceRoot: string;
    command: BuildSessionCommand;
    callback: (terminal: TaskTerminal) => Promise<void>;
  }): Promise<void> {
    this.cancellingSchemes.delete(options.scheme);
    this.startSchemeBuild(options.scheme);
    this.emitter.emit("buildSessionStarted", { scheme: options.scheme, command: options.command });
    let status: BuildSessionEnded["status"] = "succeeded";
    try {
      await runTask(this.execution, {
        workspaceRoot: options.workspaceRoot,
        name: options.name,
        lock: "sweetpad.build",
        terminateLocked: true,
        problemMatchers: BUILD_TASK_PROBLEM_MATCHERS,
        metadata: { scheme: options.scheme },
        callback: options.callback,
      });
    } catch (error) {
      status = this.cancellingSchemes.has(options.scheme) ? "cancelled" : "failed";
      throw error;
    } finally {
      this.emitter.emit("buildSessionEnded", { scheme: options.scheme, status });
      this.cancellingSchemes.delete(options.scheme);
      this.stopSchemeBuild(options.scheme);
    }
  }

  /**
   * Build app without running
   */
  async buildCommand(item: BuildTreeItem | undefined, options: { debug: boolean }) {
    this.progress.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath({
      workspaceState: this.workspaceState,
      workspaceContext: this.workspaceContext,
      buildManager: this,
    });
    const workspaceRoot = getWorkspaceRoot({
      xcworkspace: xcworkspace,
      workspaceContext: this.workspaceContext,
    });

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.progress, this, { title: "Select scheme to build", xcworkspace: xcworkspace }));

    await generateBuildServerConfigOnBuild({
      workspaceRoot: workspaceRoot,
      scheme: scheme,
      xcworkspace: xcworkspace,
      workspaceState: this.workspaceState,
    });

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.progress, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.progress, this.destinations, {
      workspaceRoot: workspaceRoot,
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });
    const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

    const sdk = destination.platform;

    await this.runSchemeTask({
      name: "Build",
      scheme: scheme,
      workspaceRoot: workspaceRoot,
      command: "build",
      callback: async (terminal) => {
        await this.buildApp(terminal, {
          workspaceRoot: workspaceRoot,
          scheme: scheme,
          sdk: sdk,
          configuration: configuration,
          shouldBuild: true,
          shouldClean: false,
          shouldTest: false,
          xcworkspace: xcworkspace,
          destinationRaw: destinationRaw,
          debug: options.debug,
        });
      },
    });
  }

  /**
   * Run application on the simulator or device without building
   */
  async runCommand(item: BuildTreeItem | undefined, options: { debug: boolean }) {
    this.progress.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath({
      workspaceState: this.workspaceState,
      workspaceContext: this.workspaceContext,
      buildManager: this,
    });
    const workspaceRoot = getWorkspaceRoot({
      xcworkspace: xcworkspace,
      workspaceContext: this.workspaceContext,
    });

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.progress, this, {
        title: "Select scheme to build and run",
        xcworkspace: xcworkspace,
      }));

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.progress, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.progress, this.destinations, {
      workspaceRoot: workspaceRoot,
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });

    const sdk = destination.platform;

    const schemeSettings = await getSchemeLaunchSettings({ xcworkspace: xcworkspace, scheme: scheme });
    const launchArgs = [...schemeSettings.args, ...(getWorkspaceConfig("build.launchArgs") ?? [])];
    const launchEnv = { ...schemeSettings.env, ...getWorkspaceConfig("build.launchEnv") };

    await this.runSchemeTask({
      name: "Run",
      scheme: scheme,
      workspaceRoot: workspaceRoot,
      command: "run",
      callback: async (terminal) => {
        if (destination.type === "macOS") {
          await this.runOnMac(terminal, {
            workspaceRoot: workspaceRoot,
            scheme: scheme,
            xcworkspace: xcworkspace,
            configuration: configuration,
            watchMarker: false,
            launchArgs: launchArgs,
            launchEnv: launchEnv,
          });
        } else if (
          destination.type === "iOSSimulator" ||
          destination.type === "watchOSSimulator" ||
          destination.type === "visionOSSimulator" ||
          destination.type === "tvOSSimulator"
        ) {
          await this.runOniOSSimulator(terminal, {
            workspaceRoot: workspaceRoot,
            scheme: scheme,
            destination: destination,
            sdk: sdk,
            configuration: configuration,
            xcworkspace: xcworkspace,
            watchMarker: false,
            launchArgs: launchArgs,
            launchEnv: launchEnv,
            debug: options.debug,
          });
        } else if (
          destination.type === "iOSDevice" ||
          destination.type === "watchOSDevice" ||
          destination.type === "tvOSDevice" ||
          destination.type === "visionOSDevice"
        ) {
          await this.runOniOSDevice(terminal, {
            workspaceRoot: workspaceRoot,
            scheme: scheme,
            destination: destination,
            sdk: sdk,
            configuration: configuration,
            xcworkspace: xcworkspace,
            watchMarker: false,
            launchArgs: launchArgs,
            launchEnv: launchEnv,
            debug: options.debug,
          });
        } else {
          assertUnreachable(destination);
        }
      },
    });
  }

  /**
   * Build and run application on the simulator or device
   */
  async launchCommand(item: BuildTreeItem | undefined, options: { debug: boolean }) {
    this.progress.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath({
      workspaceState: this.workspaceState,
      workspaceContext: this.workspaceContext,
      buildManager: this,
    });
    const workspaceRoot = getWorkspaceRoot({
      xcworkspace: xcworkspace,
      workspaceContext: this.workspaceContext,
    });

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.progress, this, {
        title: "Select scheme to build and run",
        xcworkspace: xcworkspace,
      }));

    await generateBuildServerConfigOnBuild({
      workspaceRoot: workspaceRoot,
      scheme: scheme,
      xcworkspace: xcworkspace,
      workspaceState: this.workspaceState,
    });

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.progress, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.progress, this.destinations, {
      workspaceRoot: workspaceRoot,
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });

    const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

    const sdk = destination.platform;

    const schemeSettings = await getSchemeLaunchSettings({ xcworkspace: xcworkspace, scheme: scheme });
    const launchArgs = [...schemeSettings.args, ...(getWorkspaceConfig("build.launchArgs") ?? [])];
    const launchEnv = { ...schemeSettings.env, ...getWorkspaceConfig("build.launchEnv") };

    await this.runSchemeTask({
      name: options.debug ? "Debug" : "Launch",
      scheme: scheme,
      workspaceRoot: workspaceRoot,
      command: "launch",
      callback: async (terminal) => {
        await this.buildApp(terminal, {
          workspaceRoot: workspaceRoot,
          scheme: scheme,
          sdk: sdk,
          configuration: configuration,
          shouldBuild: true,
          shouldClean: false,
          shouldTest: false,
          xcworkspace: xcworkspace,
          destinationRaw: destinationRaw,
          debug: options.debug,
        });

        if (destination.type === "macOS") {
          await this.runOnMac(terminal, {
            workspaceRoot: workspaceRoot,
            scheme: scheme,
            xcworkspace: xcworkspace,
            configuration: configuration,
            watchMarker: false,
            launchArgs: launchArgs,
            launchEnv: launchEnv,
          });
        } else if (
          destination.type === "iOSSimulator" ||
          destination.type === "watchOSSimulator" ||
          destination.type === "tvOSSimulator" ||
          destination.type === "visionOSSimulator"
        ) {
          await this.runOniOSSimulator(terminal, {
            workspaceRoot: workspaceRoot,
            scheme: scheme,
            destination: destination,
            sdk: sdk,
            configuration: configuration,
            xcworkspace: xcworkspace,
            watchMarker: false,
            launchArgs: launchArgs,
            launchEnv: launchEnv,
            debug: options.debug,
          });
        } else if (
          destination.type === "iOSDevice" ||
          destination.type === "watchOSDevice" ||
          destination.type === "tvOSDevice" ||
          destination.type === "visionOSDevice"
        ) {
          await this.runOniOSDevice(terminal, {
            workspaceRoot: workspaceRoot,
            scheme: scheme,
            destination: destination,
            sdk: sdk,
            configuration: configuration,
            xcworkspace: xcworkspace,
            watchMarker: false,
            launchArgs: launchArgs,
            launchEnv: launchEnv,
            debug: options.debug,
          });
        } else {
          assertUnreachable(destination);
        }
      },
    });
  }

  async runOnMac(
    terminal: TaskTerminal,
    options: {
      workspaceRoot: string;
      scheme: string;
      xcworkspace: string;
      configuration: string;
      watchMarker: boolean;
      launchArgs: string[];
      launchEnv: Record<string, string>;
    },
  ) {
    this.progress.updateText("Extracting build settings");
    const destinationRaw = buildDestinationString({ platform: "macOS" });
    const buildSettings = await getBuildSettingsToLaunch({
      workspaceRoot: options.workspaceRoot,
      scheme: options.scheme,
      configuration: options.configuration,
      sdk: "macosx",
      xcworkspace: options.xcworkspace,
      destination: destinationRaw,
    });

    const executablePath = await ensureAppPathExists(buildSettings.executablePath);

    this.workspaceState.update("build.lastLaunchedApp", {
      type: "macos",
      appPath: executablePath,
      bundleIdentifier: buildSettings.bundleIdentifier,
    });
    if (options.watchMarker) {
      writeWatchMarkers(terminal);
    }

    this.progress.updateText(`Running "${options.scheme}" on Mac`);
    await ensureInjectionAppRunning();
    const launchEnv = await withHotReloadLaunchEnv({
      terminal: terminal,
      state: this.workspaceState,
      launchEnv: options.launchEnv,
      destinationType: "macOS",
      workspaceRoot: options.workspaceRoot,
    });
    await terminal.runGroup(async (group) => {
      const logSidecar = new MacOSLogSidecar(group, {
        bundleId: buildSettings.bundleIdentifier,
        executableName: buildSettings.executableName,
      });
      await logSidecar.spawn();

      const main = new MainExecutable(group, {
        command: executablePath,
        args: options.launchArgs,
        // NSUnbufferedIO is a no-op when stdout is a tty (the v3/node-pty path), but acts as a
        // safety net for the v2 fallback where stdout is a plain pipe and Foundation block-buffers print().
        env: { NSUnbufferedIO: "YES", ...launchEnv },
        pty: true,
      });
      await main.wait();
    });
  }

  async runOniOSSimulator(
    terminal: TaskTerminal,
    options: {
      workspaceRoot: string;
      scheme: string;
      destination: SimulatorDestination;
      sdk: string;
      configuration: string;
      xcworkspace: string;
      watchMarker: boolean;
      launchArgs: string[];
      launchEnv: Record<string, string>;
      debug: boolean;
    },
  ) {
    const simulatorId = options.destination.udid;

    this.progress.updateText("Extracting build settings");
    const destinationRaw = getXcodeBuildDestinationString({ destination: options.destination });
    const buildSettings = await getBuildSettingsToLaunch({
      workspaceRoot: options.workspaceRoot,
      scheme: options.scheme,
      configuration: options.configuration,
      sdk: options.sdk,
      xcworkspace: options.xcworkspace,
      destination: destinationRaw,
    });
    const appPath = await ensureAppPathExists(buildSettings.appPath);
    const bundlerId = buildSettings.bundleIdentifier;

    // Get simulator with fresh state
    this.progress.updateText(`Searching for simulator "${simulatorId}"`);
    const simulator = await getSimulatorByUdid(this.destinations, {
      udid: simulatorId,
    });

    // Boot device
    if (!simulator.isBooted) {
      this.progress.updateText(`Booting simulator "${simulator.name}"`);
      await terminal.execute({
        command: "xcrun",
        args: ["simctl", "boot", simulator.udid],
      });

      // Refresh list of simulators after we start new simulator
      this.destinations.refreshSimulators();
    }

    // Open simulator
    this.progress.updateText("Launching Simulator.app");
    const bringToForeground = getWorkspaceConfig("build.bringSimulatorToForeground") ?? true;
    const openArgs = bringToForeground ? ["-a", "Simulator"] : ["-g", "-a", "Simulator"];
    await terminal.execute({
      command: "open",
      args: openArgs,
    });

    // Install app
    this.progress.updateText(`Installing "${options.scheme}" on "${simulator.name}"`);
    await terminal.execute({
      command: "xcrun",
      args: ["simctl", "install", simulator.udid, appPath],
    });

    this.workspaceState.update("build.lastLaunchedApp", {
      type: "simulator",
      appPath: appPath,
      bundleIdentifier: bundlerId,
      simulatorUdid: simulator.udid,
    });
    if (options.watchMarker) {
      writeWatchMarkers(terminal);
    }

    const launchArgs = [
      "simctl",
      "launch",
      "--console-pty",
      // This instructs app to wait for the debugger to be attached before launching,
      // ensuring you can debug issues happening early on.
      ...(options.debug ? ["--wait-for-debugger"] : []),
      "--terminate-running-process",
      simulator.udid,
      bundlerId,
      ...options.launchArgs,
    ];

    // Run app
    this.progress.updateText(`Running "${options.scheme}" on "${simulator.name}"`);
    await ensureInjectionAppRunning();
    const childEnv = await withHotReloadLaunchEnv({
      terminal: terminal,
      state: this.workspaceState,
      launchEnv: options.launchEnv,
      destinationType: options.destination.type,
      workspaceRoot: options.workspaceRoot,
    });
    await terminal.runGroup(async (group) => {
      const logSidecar = new SimulatorLogSidecar(group, {
        simulatorUdid: simulator.udid,
        bundleId: bundlerId,
        executableName: buildSettings.executableName,
      });
      await logSidecar.spawn();

      const main = new MainExecutable(group, {
        command: "xcrun",
        args: launchArgs,
        // simctl strips SIMCTL_CHILD_ and passes the rest to the launched app.
        env: Object.fromEntries(Object.entries(childEnv).map(([k, v]) => [`SIMCTL_CHILD_${k}`, v])),
        pty: true,
      });
      await main.wait();
    });
  }

  async runOniOSDevice(
    terminal: TaskTerminal,
    option: {
      workspaceRoot: string;
      scheme: string;
      configuration: string;
      destination: DeviceDestination;
      sdk: string;
      xcworkspace: string;
      watchMarker: boolean;
      launchArgs: string[];
      launchEnv: Record<string, string>;
      debug: boolean;
    },
  ) {
    const { scheme, configuration, destination } = option;
    const { type: destinationType, name: destinationName } = destination;

    this.progress.updateText("Extracting build settings");
    const destinationRaw = getXcodeBuildDestinationString({ destination: destination });
    const buildSettings = await getBuildSettingsToLaunch({
      workspaceRoot: option.workspaceRoot,
      scheme: scheme,
      configuration: configuration,
      sdk: option.sdk,
      xcworkspace: option.xcworkspace,
      destination: destinationRaw,
    });

    const targetPath = await ensureAppPathExists(buildSettings.appPath);
    const bundlerId = buildSettings.bundleIdentifier;

    const runMethod = resolveDeviceRunMethod({
      supportsDevicectl: destination.supportsDevicectl,
      debug: option.debug,
    });

    // Use appropriate device ID format for the deployment method
    // - devicectl uses the devicectl identifier format
    // - ios-deploy uses the legacy UDID format
    const deviceId = runMethod === "devicectl" ? destination.devicectlId : destination.udid;

    // Validate that we have a device ID
    if (!deviceId) {
      throw new ExtensionError(`Could not determine device ID for ${destinationName}`);
    }

    // Install and launch app on device
    this.progress.updateText(`Installing "${scheme}" on "${destinationName}"`);

    // The debugserver method emits its own marker once the port is live, so that the debug
    // session does not start connecting before there is anything to connect to.
    if (option.watchMarker && runMethod !== "ios-deploy-debugserver") {
      writeWatchMarkers(terminal);
    }

    // Launch app on device
    this.progress.updateText(`Running "${option.scheme}" on "${option.destination.name}"`);

    if (runMethod === "devicectl") {
      // Use devicectl for iOS 17+ devices - separate install and launch
      await terminal.execute({
        command: "xcrun",
        args: ["devicectl", "device", "install", "app", "--device", deviceId, targetPath],
      });

      await using jsonOutputPath = await tempFilePath(this.vscodeContext, {
        prefix: "json",
      });

      this.progress.updateText("Extracting Xcode version");
      const xcodeVersion = await getXcodeVersionInstalled({
        workspaceRoot: option.workspaceRoot,
      });
      const isConsoleOptionSupported = xcodeVersion.major >= 16;

      this.workspaceState.update("build.lastLaunchedApp", {
        type: "device",
        appPath: targetPath,
        appName: buildSettings.appName,
        executableName: buildSettings.executableName,
        bundleIdentifier: bundlerId,
        destinationId: deviceId,
        destinationType: destinationType,
      });

      // Prepare the launch arguments
      const launchArgs = [
        "devicectl",
        "device",
        "process",
        "launch",
        // Attaches the application to the console and waits for it to exit
        isConsoleOptionSupported ? "--console" : null,
        "--json-output",
        jsonOutputPath.path,
        // Terminates any already-running instances of the app prior to launch. Not supported on all platforms.
        "--terminate-existing",
        "--device",
        deviceId,
        bundlerId,
        // A `--` separator tells devicectl that everything after it belongs to
        // the launched app, not to devicectl itself. Without it, launch args
        // that start with `-` (common NSUserDefaults-style flags) are parsed as
        // devicectl options and the launch fails. See issue #296.
        ...(option.launchArgs.length ? ["--", ...option.launchArgs] : []),
      ].filter((arg) => arg !== null); // Filter out null arguments

      this.progress.updateText(`Running "${option.scheme}" on "${option.destination.name}"`);

      await this.tunnel.autoConnect();

      await terminal.runGroup(async (group) => {
        // pymobiledevice3 is the only device log backend; toggle the global
        // build.logStreamEnabled to disable. Pymd3Sidecar.spec() returns null and writes
        // a [sweetpad] warning when streaming is disabled, the binary is missing, or the
        // executable name is unknown; pymd3's own stderr (e.g. tunneld not running)
        // surfaces via [pymobiledevice3]. The launch proceeds either way.
        const logSidecar = new Pymd3Sidecar(group, {
          executableName: buildSettings.executableName,
          enableDebugDylib: buildSettings.enableDebugDylib,
        });
        await logSidecar.spawn();

        const main = new MainExecutable(group, {
          command: "xcrun",
          args: launchArgs,
          // devicectl strips DEVICECTL_CHILD_ and passes the rest to the launched app.
          env: Object.fromEntries(Object.entries(option.launchEnv).map(([k, v]) => [`DEVICECTL_CHILD_${k}`, v])),
          pty: true,
        });
        await main.wait();
      });

      let jsonOutput: any;
      try {
        jsonOutput = await readJsonFile(jsonOutputPath.path);
      } catch (e) {
        throw new ExtensionError("Error reading json output");
      }

      if (jsonOutput.info.outcome !== "success") {
        terminal.write("Error launching app on device", {
          newLine: true,
        });
        terminal.write(JSON.stringify(jsonOutput.result, null, 2), {
          newLine: true,
        });
        return;
      }
      terminal.write(`App launched on device with PID: ${jsonOutput.result.process.processIdentifier}`, {
        newLine: true,
      });
    } else if (runMethod === "ios-deploy-debugserver") {
      // Use ios-deploy for older devices (iOS < 17)
      commonLogger.debug("Using ios-deploy for older device", {
        deviceId: deviceId,
        osVersion: destination.osVersion,
        runMethod: runMethod,
      });

      await this.checkIosDeployInstalled();

      await this.launchWithDebugserver(terminal, {
        deviceId: deviceId,
        appPath: targetPath,
        appName: buildSettings.appName,
        executableName: buildSettings.executableName,
        enableDebugDylib: buildSettings.enableDebugDylib,
        bundleId: bundlerId,
        destinationType: destinationType,
        watchMarker: option.watchMarker,
        launchArgs: option.launchArgs,
        launchEnv: option.launchEnv,
      });
    } else if (runMethod === "ios-deploy") {
      // Use ios-deploy for older devices (iOS < 17)
      commonLogger.debug("Using ios-deploy for older device", {
        deviceId: deviceId,
        osVersion: destination.osVersion,
        runMethod: runMethod,
      });

      await this.checkIosDeployInstalled();

      // ios-deploy handles both install and launch in one command with --debug
      this.workspaceState.update("build.lastLaunchedApp", {
        type: "device",
        appPath: targetPath,
        appName: buildSettings.appName,
        executableName: buildSettings.executableName,
        bundleIdentifier: bundlerId,
        destinationId: deviceId,
        destinationType: destinationType,
      });

      await terminal.runGroup(async (group) => {
        // ios-deploy relays the app's stdout/stderr into the terminal itself, but os_log and
        // Logger never reach stdout — reading those needs a connection to the device's
        // syslog, same as on the devicectl path.
        const logSidecar = new Pymd3Sidecar(group, {
          executableName: buildSettings.executableName,
          enableDebugDylib: buildSettings.enableDebugDylib,
        });
        await logSidecar.spawn();

        // Runs through terminal.execute rather than the group: ios-deploy stays the
        // foreground process, so Ctrl+C reaches it and not the sidecar.
        await iosDeploy.installAndLaunchApp(this.vscodeContext, terminal, {
          deviceId: deviceId,
          appPath: targetPath,
          bundleId: bundlerId,
          launchArgs: option.launchArgs,
          launchEnv: option.launchEnv,
        });
      });

      terminal.write("App launched on device", {
        newLine: true,
      });
    } else {
      assertUnreachable(runMethod);
    }
  }

  private async checkIosDeployInstalled() {
    // Check if ios-deploy is installed before attempting to use it
    const isInstalled = await iosDeploy.isIosDeployInstalled();
    if (!isInstalled) {
      throw new ExtensionError("ios-deploy is required for iOS < 17. Install it with: brew install ios-deploy");
    }
  }

  /**
   * Install the app on a device without CoreDevice (iOS 16 and below) and hand LLDB a
   * debugserver to connect to.
   *
   * The ordering here is load-bearing. ios-deploy has to reach its debug phase before the
   * watch marker releases the "sweetpad: debugging-launch" pre-launch task, because that
   * marker is what lets the debug session start resolving its configuration — and by then
   * the port it connects to must already be in the workspace state.
   */
  private async launchWithDebugserver(
    terminal: TaskTerminal,
    options: {
      deviceId: string;
      appPath: string;
      appName: string;
      executableName?: string;
      enableDebugDylib: boolean;
      bundleId: string;
      destinationType: DestinationType;
      watchMarker: boolean;
      launchArgs: string[];
      launchEnv: Record<string, string>;
    },
  ) {
    commonLogger.debug("Starting ios-deploy debugserver", {
      deviceId: options.deviceId,
      appPath: options.appPath,
    });

    await terminal.runGroup(async (group) => {
      const session = await iosDeploy.launchDebugserver(group, {
        deviceId: options.deviceId,
        appPath: options.appPath,
      });

      // Started before LLDB launches the app, so startup logging is not missed. No tunnel
      // is set up first, unlike the devicectl path: pymobiledevice3 reaches syslog over
      // usbmux, and RemoteXPC — the thing that needs a tunnel — is iOS 17 and up.
      const logSidecar = new Pymd3Sidecar(group, {
        executableName: options.executableName,
        enableDebugDylib: options.enableDebugDylib,
      });
      await logSidecar.spawn();

      this.workspaceState.update("build.lastLaunchedApp", {
        type: "device",
        appPath: options.appPath,
        appName: options.appName,
        executableName: options.executableName,
        bundleIdentifier: options.bundleId,
        destinationId: options.deviceId,
        destinationType: options.destinationType,
        debugserver: {
          port: session.debugserver.port,
          deviceAppPath: session.debugserver.deviceAppPath,
          symbolsPath: session.debugserver.symbolsPath,
          launchArgs: options.launchArgs,
          launchEnv: options.launchEnv,
        },
      });

      terminal.write(`Debugserver listening on 127.0.0.1:${session.debugserver.port}`, {
        newLine: true,
      });
      if (options.watchMarker) {
        writeWatchMarkers(terminal);
      }

      // The app's stdout/stderr travel over the debugserver connection to the Debug Console;
      // os_log/Logger output reaches this terminal through the sidecar above. Holding this
      // open is what keeps the debug session alive.
      await session.wait();
    });
  }

  async buildApp(
    terminal: TaskTerminal,
    options: {
      workspaceRoot: string;
      scheme: string;
      sdk: string;
      configuration: string;
      shouldBuild: boolean;
      shouldClean: boolean;
      shouldTest: boolean;
      xcworkspace: string;
      destinationRaw: string;
      debug: boolean;
    },
  ) {
    const useXcbeautify = isXcbeautifyEnabled() && (await getIsXcbeautifyInstalled());
    const bundlePath = await prepareBundleDir(this.vscodeContext, options.scheme);
    const derivedDataPath = prepareDerivedDataPath({
      workspaceRoot: options.workspaceRoot,
    });

    const arch = getWorkspaceConfig("build.arch") || undefined;
    const allowProvisioningUpdates = getWorkspaceConfig("build.allowProvisioningUpdates") ?? true;

    // ex: ["-arg1", "value1", "-arg2", "value2", "-arg3", "-arg4", "value4"]
    const additionalArgs: string[] = getWorkspaceConfig("build.args") || [];

    // ex: { "ARG1": "value1", "ARG2": null, "ARG3": "value3" }
    const env = getWorkspaceConfig("build.env") || {};

    const workspaceType = detectWorkspaceType(options.xcworkspace);

    const command = new XcodeCommandBuilder();
    if (arch) {
      command.addBuildSettings("ARCHS", arch);
      command.addBuildSettings("VALID_ARCHS", arch);
      command.addBuildSettings("ONLY_ACTIVE_ARCH", "NO");
    }

    // Add debug-specific build settings if in debug mode
    if (options.debug) {
      // This tells the compiler to generate debugging symbols and include them in the compiled binary.
      // Without this, LLDB wont know how to match lines of code to machine instructions. This is normally
      // set to YES on XCode debug builds, but forcing it here, ensures you'll always get them in
      // sweetpad: debugging-launch
      command.addBuildSettings("GCC_GENERATE_DEBUGGING_SYMBOLS", "YES");
      // In Xcode, ONLY_ACTIVE_ARCH is a build setting that controls whether you compile for only the architecture
      // of the machine (or simulator/device) you're currently targeting, or for all architectures listed in your
      // project's ARCHS setting.
      // It speeds up compile times, especially in Debug, because Xcode skips generating unused slices.
      command.addBuildSettings("ONLY_ACTIVE_ARCH", "YES");
    }

    // InjectionNext needs `-Xlinker -interposable` so dyld can swap symbols at runtime,
    // and EMIT_FRONTEND_COMMAND_LINES=YES so it can recover compile commands from the
    // build logs when no Xcode IDE is supervising the build (required for Xcode 16.3+).
    // $(inherited) keeps whatever the project already sets for OTHER_LDFLAGS. Skipped
    // for SDKs that InjectionNext can't inject into (physical devices, watchOS), so
    // device builds don't pay for the extra relocations.
    if (isHotReloadEnabled() && sdkSupportsHotReload(options.sdk)) {
      command.addBuildSettings("OTHER_LDFLAGS", "$(inherited) -Xlinker -interposable");
      command.addBuildSettings("EMIT_FRONTEND_COMMAND_LINES", "YES");
    }

    command.addParameters("-scheme", options.scheme);
    command.addParameters("-configuration", options.configuration);
    command.addParameters("-destination", options.destinationRaw);
    command.addParameters("-resultBundlePath", bundlePath);
    if (derivedDataPath) {
      command.addParameters("-derivedDataPath", derivedDataPath);
    }
    if (allowProvisioningUpdates) {
      command.addOption("-allowProvisioningUpdates");
    }

    // Add workspace parameter only for Xcode projects
    if (workspaceType === "xcode") {
      command.addParameters("-workspace", options.xcworkspace);
    }

    if (options.shouldClean) {
      command.addAction("clean");
    }
    if (options.shouldBuild) {
      command.addAction("build");
    }
    if (options.shouldTest) {
      command.addAction("test");
    }
    command.addAdditionalArgs(additionalArgs);

    const commandParts = command.build();
    let pipes: Command[] | undefined = undefined;
    if (useXcbeautify) {
      pipes = [{ command: "xcbeautify", args: [] }];
    }

    if (options.shouldClean) {
      this.progress.updateText(`Cleaning "${options.scheme}"`);
    } else if (options.shouldBuild) {
      this.progress.updateText(`Building "${options.scheme}"`);
    } else if (options.shouldTest) {
      this.progress.updateText(`Building "${options.scheme}"`);
    }

    await generateBuildServerConfigOnBuild({
      workspaceRoot: options.workspaceRoot,
      scheme: options.scheme,
      xcworkspace: options.xcworkspace,
      workspaceState: this.workspaceState,
    });

    let cwd: string;
    if (workspaceType === "spm") {
      cwd = getSwiftPMDirectory(options.xcworkspace);
    } else if (workspaceType === "xcode") {
      cwd = options.workspaceRoot;
    } else {
      assertUnreachable(workspaceType);
    }

    const diagnostics = this.diagnostics.beginBuild({ mode: useXcbeautify ? "xcbeautify" : "xcodebuild" });
    try {
      await terminal.execute({
        command: commandParts[0],
        args: commandParts.slice(1),
        pipes: pipes,
        env: env,
        cwd: cwd,
        closeStdin: true,
        onOutputLine: async ({ value }) => {
          const parsed = diagnostics.recordLine(value);
          this.emitter.emit("buildLogLine", { line: value, diagnostic: parsed });
        },
      });
    } finally {
      diagnostics.flush();
    }

    await restartSwiftLSP();
  }

  async cleanCommand(item: BuildTreeItem | undefined) {
    this.progress.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath({
      workspaceState: this.workspaceState,
      workspaceContext: this.workspaceContext,
      buildManager: this,
    });
    const workspaceRoot = getWorkspaceRoot({
      xcworkspace: xcworkspace,
      workspaceContext: this.workspaceContext,
    });

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.progress, this, { title: "Select scheme to clean", xcworkspace: xcworkspace }));

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.progress, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.progress, this.destinations, {
      workspaceRoot: workspaceRoot,
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });
    const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

    const sdk = destination.platform;

    await this.runSchemeTask({
      name: "Clean",
      scheme: scheme,
      workspaceRoot: workspaceRoot,
      command: "clean",
      callback: async (terminal) => {
        await this.buildApp(terminal, {
          workspaceRoot: workspaceRoot,
          scheme: scheme,
          sdk: sdk,
          configuration: configuration,
          shouldBuild: false,
          shouldClean: true,
          shouldTest: false,
          xcworkspace: xcworkspace,
          destinationRaw: destinationRaw,
          debug: false,
        });
      },
    });
  }

  async testCommand(item: BuildTreeItem | undefined) {
    this.progress.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath({
      workspaceState: this.workspaceState,
      workspaceContext: this.workspaceContext,
      buildManager: this,
    });
    const workspaceRoot = getWorkspaceRoot({
      xcworkspace: xcworkspace,
      workspaceContext: this.workspaceContext,
    });

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.progress, this, { title: "Select scheme to test", xcworkspace: xcworkspace }));

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.progress, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.progress, this.destinations, {
      workspaceRoot: workspaceRoot,
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });
    const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

    const sdk = destination.platform;

    await this.runSchemeTask({
      name: "Test",
      scheme: scheme,
      workspaceRoot: workspaceRoot,
      command: "test",
      callback: async (terminal) => {
        await this.buildApp(terminal, {
          workspaceRoot: workspaceRoot,
          scheme: scheme,
          sdk: sdk,
          configuration: configuration,
          shouldBuild: false,
          shouldClean: false,
          shouldTest: true,
          xcworkspace: xcworkspace,
          destinationRaw: destinationRaw,
          debug: false,
        });
      },
    });
  }

  async resolveDependenciesCommand(options: { scheme: string; xcworkspace: string }): Promise<void> {
    this.progress.updateText("Resolving dependencies");

    const workspaceRoot = getWorkspaceRoot({
      xcworkspace: options.xcworkspace,
      workspaceContext: this.workspaceContext,
    });

    await this.runSchemeTask({
      name: "Resolve Dependencies",
      scheme: options.scheme,
      workspaceRoot: workspaceRoot,
      command: "resolve-deps",
      callback: async (terminal) => {
        const workspaceType = detectWorkspaceType(options.xcworkspace);
        if (workspaceType === "spm") {
          const packageDir = getSwiftPMDirectory(options.xcworkspace);
          await terminal.execute({
            command: getSwiftCommand(),
            args: ["package", "resolve"],
            cwd: packageDir,
          });
        } else if (workspaceType === "xcode") {
          await terminal.execute({
            command: getXcodeBuildCommand(),
            args: ["-resolvePackageDependencies", "-scheme", options.scheme, "-workspace", options.xcworkspace],
            closeStdin: true,
          });
        } else {
          assertUnreachable(workspaceType);
        }
      },
    });
  }

  async stopSchemeCommand(item: BuildTreeItem | undefined): Promise<void> {
    const scheme = item?.scheme;
    if (!scheme) return;
    await this.stopScheme(scheme);
  }

  async stopScheme(scheme: string): Promise<void> {
    this.cancellingSchemes.add(scheme);
    const tasks = vscode.tasks.taskExecutions.filter(
      ({ task }) => task.definition.lockId === "sweetpad.build" && task.definition.metadata?.scheme === scheme,
    );
    for (const task of tasks) {
      task.terminate();
    }
    this.stopSchemeBuild(scheme);
  }

  getRunningScheme(): string | undefined {
    return [...this.runningSchemes][0];
  }
}
