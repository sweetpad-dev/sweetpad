import events from "node:events";
import * as path from "node:path";

import * as vscode from "vscode";

import {
  type XcodeScheme,
  getBuildSettingsToLaunch,
  getIsXcbeautifyInstalled,
  getIsXcodeBuildServerInstalled,
  getSchemes,
  getSwiftCommand,
  getXcodeBuildCommand,
  getXcodeVersionInstalled,
} from "../common/cli/scripts";
import { getWorkspaceConfig } from "../common/config";
import { ExtensionError } from "../common/errors";
import { BaseExecutionScope, type ExecutionScopeService } from "../common/execution-scope";
import { isFileExists, readJsonFile, tempFilePath } from "../common/files";
import { commonLogger } from "../common/logger";
import { runTask } from "../common/tasks/run";
import type { Command, TaskTerminal } from "../common/tasks/types";
import { assertUnreachable } from "../common/types";
import type { WorkspaceStateService } from "../common/workspace-state";
import * as iosDeploy from "../common/xcode/ios-deploy";
import type { DestinationsManager } from "../destination/manager";
import type { TunnelManager } from "../devices/tunnel";
import type { DeviceDestination } from "../devices/types";
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
  getCurrentXcodeWorkspacePath,
  notifyXcodeBuildServerMissing,
  getSchemeLaunchSettings,
  getSwiftPMDirectory,
  getWorkspacePath,
  getXcodeBuildDestinationString,
  isAutoGenerateBuildServerConfigEnabled,
  isXcbeautifyEnabled,
  prepareBundleDir,
  prepareDerivedDataPath,
  refreshBuildServer,
  restartSwiftLSP,
  writeWatchMarkers,
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
  private workspace: WorkspaceStateService;
  private progress: ProgressStatusBar;
  private execution: ExecutionScopeService;
  private tunnel: TunnelManager;
  private vscodeContext: vscode.ExtensionContext;
  private destinations: DestinationsManager;
  private diagnostics: DiagnosticsManager;
  private runningSchemes: Set<string> = new Set();
  private cancellingSchemes: Set<string> = new Set();

  constructor(options: {
    workspace: WorkspaceStateService;
    progress: ProgressStatusBar;
    execution: ExecutionScopeService;
    tunnel: TunnelManager;
    vscodeContext: vscode.ExtensionContext;
    destinations: DestinationsManager;
    diagnostics: DiagnosticsManager;
  }) {
    this.workspace = options.workspace;
    this.progress = options.progress;
    this.execution = options.execution;
    this.tunnel = options.tunnel;
    this.vscodeContext = options.vscodeContext;
    this.destinations = options.destinations;
    this.diagnostics = options.diagnostics;
  }

  async start(): Promise<void> {
    this.on("defaultSchemeForBuildUpdated", (scheme: string | undefined) => {
      void this.generateXcodeBuildServerSettingsOnSchemeChange({
        scheme: scheme,
      });
    });
  }

  on<K extends IEventKey>(event: K, listener: (...args: IEventMap[K]) => void): void {
    this.emitter.on(event, listener as any); // todo: fix this any
  }

  off<K extends IEventKey>(event: K, listener: (...args: IEventMap[K]) => void): void {
    this.emitter.off(event, listener as any);
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
        const xcworkspace = getCurrentXcodeWorkspacePath(this.workspace);

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
    return this.workspace.get("build.xcodeScheme");
  }

  getDefaultSchemeForTesting(): string | undefined {
    return this.workspace.get("testing.xcodeScheme");
  }

  setDefaultSchemeForBuild(scheme: string | undefined): void {
    this.workspace.update("build.xcodeScheme", scheme);
    this.emitter.emit("defaultSchemeForBuildUpdated", scheme);
  }

  setDefaultSchemeForTesting(scheme: string | undefined): void {
    this.workspace.update("testing.xcodeScheme", scheme);
    this.emitter.emit("defaultSchemeForTestingUpdated", scheme);
  }

  getDefaultConfigurationForBuild(): string | undefined {
    return this.workspace.get("build.xcodeConfiguration");
  }

  getDefaultConfigurationForTesting(): string | undefined {
    return this.workspace.get("testing.xcodeConfiguration");
  }

  setDefaultConfigurationForBuild(configuration: string | undefined): void {
    this.workspace.update("build.xcodeConfiguration", configuration);
    this.emitter.emit("defaultConfigurationForBuildUpdated", configuration);
  }

  setDefaultConfigurationForTesting(configuration: string | undefined): void {
    this.workspace.update("testing.xcodeConfiguration", configuration);
  }

  /**
   * Every time the scheme changes, we need to rebuild the buildServer.json file
   * for providing the correct build settings to the LSP server.
   */
  async generateXcodeBuildServerSettingsOnSchemeChange(options: { scheme: string | undefined }): Promise<void> {
    if (!options.scheme) {
      return;
    }

    if (!isAutoGenerateBuildServerConfigEnabled()) {
      return;
    }

    const buildServerJsonPath = path.join(getWorkspacePath(), "buildServer.json");
    const isBuildServerJsonExists = await isFileExists(buildServerJsonPath);
    if (!isBuildServerJsonExists) {
      return;
    }

    const isServerInstalled = await getIsXcodeBuildServerInstalled();
    if (!isServerInstalled) {
      await notifyXcodeBuildServerMissing(this.workspace);
      return;
    }

    const xcworkspace = await askXcodeWorkspacePath(this.workspace, this);
    await refreshBuildServer({
      xcworkspace: xcworkspace,
      scheme: options.scheme,
    });

    const isShown = this.workspace.get("build.xcodeBuildServerAutogenreateInfoShown") ?? false;
    if (!isShown) {
      this.workspace.update("build.xcodeBuildServerAutogenreateInfoShown", true);
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
    const currentBuildScheme = this.getDefaultSchemeForBuild();
    if (currentBuildScheme && !schemeNames.has(currentBuildScheme)) {
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
    command: BuildSessionCommand;
    callback: (terminal: TaskTerminal) => Promise<void>;
  }): Promise<void> {
    this.cancellingSchemes.delete(options.scheme);
    this.startSchemeBuild(options.scheme);
    this.emitter.emit("buildSessionStarted", { scheme: options.scheme, command: options.command });
    let status: BuildSessionEnded["status"] = "succeeded";
    try {
      await runTask(this.execution, {
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
    const xcworkspace = await askXcodeWorkspacePath(this.workspace, this);

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.progress, this, { title: "Select scheme to build", xcworkspace: xcworkspace }));

    await generateBuildServerConfigOnBuild({
      scheme: scheme,
      xcworkspace: xcworkspace,
      workspace: this.workspace,
    });

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.progress, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.progress, this.destinations, {
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
      command: "build",
      callback: async (terminal) => {
        await this.buildApp(terminal, {
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
    const xcworkspace = await askXcodeWorkspacePath(this.workspace, this);

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
      command: "run",
      callback: async (terminal) => {
        if (destination.type === "macOS") {
          await this.runOnMac(terminal, {
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
            scheme: scheme,
            destination: destination,
            sdk: sdk,
            configuration: configuration,
            xcworkspace: xcworkspace,
            watchMarker: false,
            launchArgs: launchArgs,
            launchEnv: launchEnv,
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
    const xcworkspace = await askXcodeWorkspacePath(this.workspace, this);

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.progress, this, {
        title: "Select scheme to build and run",
        xcworkspace: xcworkspace,
      }));

    await generateBuildServerConfigOnBuild({
      scheme: scheme,
      xcworkspace: xcworkspace,
      workspace: this.workspace,
    });

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.progress, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.progress, this.destinations, {
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
      command: "launch",
      callback: async (terminal) => {
        await this.buildApp(terminal, {
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
            scheme: scheme,
            destination: destination,
            sdk: sdk,
            configuration: configuration,
            xcworkspace: xcworkspace,
            watchMarker: false,
            launchArgs: launchArgs,
            launchEnv: launchEnv,
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
      scheme: options.scheme,
      configuration: options.configuration,
      sdk: "macosx",
      xcworkspace: options.xcworkspace,
      destination: destinationRaw,
    });

    const executablePath = await ensureAppPathExists(buildSettings.executablePath);

    this.workspace.update("build.lastLaunchedApp", {
      type: "macos",
      appPath: executablePath,
      bundleIdentifier: buildSettings.bundleIdentifier,
    });
    if (options.watchMarker) {
      writeWatchMarkers(terminal);
    }

    this.progress.updateText(`Running "${options.scheme}" on Mac`);
    await ensureInjectionAppRunning();
    const launchEnv = await withHotReloadLaunchEnv(terminal, this.workspace, options.launchEnv, "macOS");
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

    this.workspace.update("build.lastLaunchedApp", {
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
    const childEnv = await withHotReloadLaunchEnv(
      terminal,
      this.workspace,
      options.launchEnv,
      options.destination.type,
    );
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
      scheme: string;
      configuration: string;
      destination: DeviceDestination;
      sdk: string;
      xcworkspace: string;
      watchMarker: boolean;
      launchArgs: string[];
      launchEnv: Record<string, string>;
    },
  ) {
    const { scheme, configuration, destination } = option;
    const { type: destinationType, name: destinationName } = destination;

    this.progress.updateText("Extracting build settings");
    const destinationRaw = getXcodeBuildDestinationString({ destination: destination });
    const buildSettings = await getBuildSettingsToLaunch({
      scheme: scheme,
      configuration: configuration,
      sdk: option.sdk,
      xcworkspace: option.xcworkspace,
      destination: destinationRaw,
    });

    const targetPath = await ensureAppPathExists(buildSettings.appPath);
    const bundlerId = buildSettings.bundleIdentifier;

    // Determine which deployment method to use based on device capabilities
    const useDevicectl = destination.supportsDevicectl;

    // Use appropriate device ID format for the deployment method
    // - devicectl uses the devicectl identifier format
    // - ios-deploy uses the legacy UDID format
    const deviceId = useDevicectl ? destination.devicectlId : destination.udid;

    // Validate that we have a device ID
    if (!deviceId) {
      throw new ExtensionError(`Could not determine device ID for ${destinationName}`);
    }

    // Install and launch app on device
    this.progress.updateText(`Installing "${scheme}" on "${destinationName}"`);

    if (option.watchMarker) {
      writeWatchMarkers(terminal);
    }

    // Launch app on device
    this.progress.updateText(`Running "${option.scheme}" on "${option.destination.name}"`);

    if (useDevicectl) {
      // Use devicectl for iOS 17+ devices - separate install and launch
      await terminal.execute({
        command: "xcrun",
        args: ["devicectl", "device", "install", "app", "--device", deviceId, targetPath],
      });

      await using jsonOutputPath = await tempFilePath(this.vscodeContext, {
        prefix: "json",
      });

      this.progress.updateText("Extracting Xcode version");
      const xcodeVersion = await getXcodeVersionInstalled();
      const isConsoleOptionSupported = xcodeVersion.major >= 16;

      this.workspace.update("build.lastLaunchedApp", {
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
        ...option.launchArgs,
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
    } else {
      // Use ios-deploy for older devices (iOS < 17)
      // ios-deploy handles both install and launch in one command with --debug
      commonLogger.debug("Using ios-deploy for older device", {
        deviceId: deviceId,
        osVersion: destination.osVersion,
      });

      // Check if ios-deploy is installed before attempting to use it
      const isInstalled = await iosDeploy.isIosDeployInstalled();
      if (!isInstalled) {
        throw new ExtensionError("ios-deploy is required for iOS < 17. Install it with: brew install ios-deploy");
      }

      this.workspace.update("build.lastLaunchedApp", {
        type: "device",
        appPath: targetPath,
        appName: buildSettings.appName,
        executableName: buildSettings.executableName,
        bundleIdentifier: bundlerId,
        destinationId: deviceId,
        destinationType: destinationType,
      });

      await iosDeploy.installAndLaunchApp(this.vscodeContext, terminal, {
        deviceId: deviceId,
        appPath: targetPath,
        bundleId: bundlerId,
        launchArgs: option.launchArgs,
        launchEnv: option.launchEnv,
      });

      terminal.write("App launched on device", {
        newLine: true,
      });
    }
  }

  async buildApp(
    terminal: TaskTerminal,
    options: {
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
    const derivedDataPath = prepareDerivedDataPath();

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
      scheme: options.scheme,
      xcworkspace: options.xcworkspace,
      workspace: this.workspace,
    });

    let cwd: string;
    if (workspaceType === "spm") {
      cwd = getSwiftPMDirectory(options.xcworkspace);
    } else if (workspaceType === "xcode") {
      cwd = getWorkspacePath();
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
    const xcworkspace = await askXcodeWorkspacePath(this.workspace, this);

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.progress, this, { title: "Select scheme to clean", xcworkspace: xcworkspace }));

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.progress, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.progress, this.destinations, {
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
      command: "clean",
      callback: async (terminal) => {
        await this.buildApp(terminal, {
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
    const xcworkspace = await askXcodeWorkspacePath(this.workspace, this);

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.progress, this, { title: "Select scheme to test", xcworkspace: xcworkspace }));

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.progress, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.progress, this.destinations, {
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
      command: "test",
      callback: async (terminal) => {
        await this.buildApp(terminal, {
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

    await this.runSchemeTask({
      name: "Resolve Dependencies",
      scheme: options.scheme,
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
