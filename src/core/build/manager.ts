import events from "node:events";
import * as path from "node:path";

import type { UserAsker } from "../asker/types";
import {
  type XcodeCliDeps,
  type XcodeScheme,
  detectWorkspaceType,
  getBasicProjectInfo,
  getBuildSettingsToLaunch,
  getIsXcbeautifyInstalled,
  getIsXcodeBuildServerInstalled,
  getSchemes,
  getSwiftCommand,
  getSwiftPMDirectory,
  getXcodeBuildCommand,
  getXcodeVersionInstalled,
} from "../cli/scripts";
import type { ConfigProvider } from "../config/types";
import type { DestinationsManager } from "../destination/manager";
import type { Destination } from "../destination/types";
import type { DeviceDestination } from "../devices/types";
import { ExtensionError } from "../errors";
import { isFileExists, readJsonFile, tempFilePath } from "../files";
import type { Logger } from "../logger/types";
import type { LspRefresher } from "../lsp/types";
import type { Notifier } from "../notifier/types";
import type { ProgressReporter } from "../progress";
import { MainExecutable } from "../run/main";
import { MacOSLogSidecar, Pymd3Sidecar, SimulatorLogSidecar } from "../run/sidecars";
import type { SimulatorDestination } from "../simulators/types";
import { getSimulatorByUdid } from "../simulators/utils";
import type { WorkspaceState } from "../state/types";
import type { Command, TaskRunner, TaskTerminal } from "../tasks/types";
import { assertUnreachable } from "../types";
import type { WorkspaceRoot } from "../workspace-root";
import * as iosDeploy from "../xcode/ios-deploy";
import { BUILD_TASK_PROBLEM_MATCHERS } from "./constants";
import type { DiagnosticsCollector } from "./diagnostics-types";
import {
  XcodeCommandBuilder,
  askConfiguration,
  askDestinationToRunOn,
  askSchemeForBuild,
  askXcodeWorkspacePath,
  ensureAppPathExists,
  generateBuildServerConfigOnBuild,
  getCurrentXcodeWorkspacePath,
  getSchemeLaunchSettings,
  getXcodeBuildDestinationString,
  isXcbeautifyEnabled,
  prepareBundleDir,
  prepareDerivedDataPath,
  refreshBuildServer,
  writeWatchMarkers,
} from "./utils";

type IEventMap = {
  refreshSchemesStarted: [];
  refreshSchemesCompleted: [XcodeScheme[]];
  refreshSchemesFailed: [];

  defaultSchemeForBuildUpdated: [scheme: string | undefined];
  defaultSchemeForTestingUpdated: [scheme: string | undefined];

  schemeBuildStarted: [scheme: string];
  schemeBuildStopped: [scheme: string];
};
type IEventKey = keyof IEventMap;

export type BuildManagerDeps = {
  logger: Logger;
  config: ConfigProvider;
  state: WorkspaceState;
  asker: UserAsker;
  progress: ProgressReporter;
  taskRunner: TaskRunner;
  notifier: Notifier;
  lsp: LspRefresher;
  destinations: DestinationsManager;
  diagnostics: DiagnosticsCollector;
  /**
   * Resolves the project root and engine storage path lazily. Deferring lets
   * `activate()` succeed on a swift-file-only window (no folder open); the
   * error only surfaces if the user actually invokes a build command.
   */
  workspaceRoot: WorkspaceRoot;
  /** Optional callback before launching on a device (e.g. tunnel auto-connect). */
  beforeDeviceLaunch?: () => Promise<void>;
};

export class BuildManager {
  private cache: XcodeScheme[] | undefined = undefined;
  private emitter = new events.EventEmitter<IEventMap>();
  private logger: Logger;
  private config: ConfigProvider;
  private state: WorkspaceState;
  private asker: UserAsker;
  private progress: ProgressReporter;
  private taskRunner: TaskRunner;
  private notifier: Notifier;
  private lsp: LspRefresher;
  private destinations: DestinationsManager;
  private diagnostics: DiagnosticsCollector;
  private workspaceRoot: WorkspaceRoot;
  private beforeDeviceLaunch?: () => Promise<void>;
  private runningSchemes: Set<string> = new Set();

  constructor(options: BuildManagerDeps) {
    this.logger = options.logger;
    this.config = options.config;
    this.state = options.state;
    this.asker = options.asker;
    this.progress = options.progress;
    this.taskRunner = options.taskRunner;
    this.notifier = options.notifier;
    this.lsp = options.lsp;
    this.destinations = options.destinations;
    this.diagnostics = options.diagnostics;
    this.workspaceRoot = options.workspaceRoot;
    this.beforeDeviceLaunch = options.beforeDeviceLaunch;
  }

  /** Snapshot of the engine deps that drive xcodebuild / swift invocations. */
  private get xcodeCli(): XcodeCliDeps {
    return { cwd: this.workspaceRoot.getPath(), config: this.config, logger: this.logger };
  }

  private get askDeps() {
    return {
      ...this.xcodeCli,
      asker: this.asker,
      progress: this.progress,
      state: this.state,
    };
  }

  async start(): Promise<void> {
    this.on("defaultSchemeForBuildUpdated", (scheme: string | undefined) => {
      void this.generateXcodeBuildServerSettingsOnSchemeChange({
        scheme: scheme,
      });
    });
  }

  on<K extends IEventKey>(event: K, listener: (...args: IEventMap[K]) => void): void {
    this.emitter.on(event, listener as any);
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
    this.progress.updateText("Refreshing Xcode schemes");

    this.emitter.emit("refreshSchemesStarted");
    try {
      getBasicProjectInfo.clearCache();

      const xcworkspace = getCurrentXcodeWorkspacePath({
        config: this.config,
        state: this.state,
        cwd: this.workspaceRoot.getPath(),
      });

      const schemes = await getSchemes(this.xcodeCli, { xcworkspace: xcworkspace });

      this.cache = schemes;

      await this.validateDefaultSchemes();
      this.emitter.emit("refreshSchemesCompleted", schemes);
      return this.cache;
    } catch (error: unknown) {
      this.logger.error("Failed to refresh schemes", { error: error });
      this.emitter.emit("refreshSchemesFailed");
      throw error;
    }
  }

  async getSchemes(options?: { refresh?: boolean }): Promise<XcodeScheme[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refreshSchemes();
    }
    return this.cache;
  }

  getDefaultSchemeForBuild(): string | undefined {
    return this.state.get("build.xcodeScheme");
  }

  getDefaultSchemeForTesting(): string | undefined {
    return this.state.get("testing.xcodeScheme");
  }

  setDefaultSchemeForBuild(scheme: string | undefined): void {
    this.state.update("build.xcodeScheme", scheme);
    this.emitter.emit("defaultSchemeForBuildUpdated", scheme);
  }

  setDefaultSchemeForTesting(scheme: string | undefined): void {
    this.state.update("testing.xcodeScheme", scheme);
    this.emitter.emit("defaultSchemeForTestingUpdated", scheme);
  }

  getDefaultConfigurationForBuild(): string | undefined {
    return this.state.get("build.xcodeConfiguration");
  }

  getDefaultConfigurationForTesting(): string | undefined {
    return this.state.get("testing.xcodeConfiguration");
  }

  setDefaultConfigurationForBuild(configuration: string | undefined): void {
    this.state.update("build.xcodeConfiguration", configuration);
  }

  setDefaultConfigurationForTesting(configuration: string | undefined): void {
    this.state.update("testing.xcodeConfiguration", configuration);
  }

  /**
   * Every time the scheme changes, we need to rebuild the buildServer.json file
   * for providing the correct build settings to the LSP server.
   */
  async generateXcodeBuildServerSettingsOnSchemeChange(options: { scheme: string | undefined }): Promise<void> {
    if (!options.scheme) {
      return;
    }

    const buildServerJsonPath = path.join(this.workspaceRoot.getPath(), "buildServer.json");
    const isBuildServerJsonExists = await isFileExists(buildServerJsonPath);
    if (!isBuildServerJsonExists) {
      return;
    }

    const isServerInstalled = await getIsXcodeBuildServerInstalled(this.xcodeCli);
    if (!isServerInstalled) {
      return;
    }

    const xcworkspace = await askXcodeWorkspacePath(this.askDeps, this);
    await refreshBuildServer(
      { ...this.xcodeCli, lsp: this.lsp },
      {
        xcworkspace: xcworkspace,
        scheme: options.scheme,
      },
    );

    const isShown = this.state.get("build.xcodeBuildServerAutogenreateInfoShown") ?? false;
    if (!isShown) {
      this.state.update("build.xcodeBuildServerAutogenreateInfoShown", true);
      this.notifier.info(
        '"buildServer.json" file is automatically regenerated every time you change the scheme. Disable this in settings if not wanted. (Shown once.)',
      );
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

  /**
   * Wrap "runTask" common options for all scheme-related tasks/actions like build, run, test,
   * etc to avoid code duplication and have a single place to update common options in the
   * future
   */
  async runSchemeTask(options: {
    name: string;
    scheme: string;
    callback: (terminal: TaskTerminal) => Promise<void>;
  }): Promise<void> {
    this.startSchemeBuild(options.scheme);
    try {
      await this.taskRunner.run({
        name: options.name,
        lock: "sweetpad.build",
        terminateLocked: true,
        problemMatchers: BUILD_TASK_PROBLEM_MATCHERS,
        metadata: { scheme: options.scheme },
        callback: options.callback,
      });
    } finally {
      this.stopSchemeBuild(options.scheme);
    }
  }

  /**
   * Build app without running
   */
  async buildCommand(item: { scheme?: string } | undefined, options: { debug: boolean }) {
    const spec = await this.resolveBuildSpec(item);
    await this.buildExplicit({
      scheme: spec.scheme,
      configuration: spec.configuration,
      destination: spec.destination,
      xcworkspace: spec.xcworkspace,
      debug: options.debug,
    });
  }

  /**
   * Ask for whatever is missing — workspace, scheme, configuration,
   * destination — and return the fully resolved tuple. Split out of
   * `buildCommand` so the VS Code extension can do the asking in-proc
   * (needs QuickPick) and then dispatch execution via either the in-proc
   * `buildExplicit` or the standalone server.
   */
  async resolveBuildSpec(item: { scheme?: string } | undefined): Promise<{
    xcworkspace: string;
    scheme: string;
    configuration: string;
    destination: Destination;
  }> {
    this.progress.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.askDeps, this);

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.askDeps, this, { title: "Select scheme to build", xcworkspace: xcworkspace }));

    await generateBuildServerConfigOnBuild(
      { ...this.xcodeCli, lsp: this.lsp },
      {
        scheme: scheme,
        xcworkspace: xcworkspace,
      },
    );

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.askDeps, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.askDeps, this.destinations, {
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });

    return { xcworkspace, scheme, configuration, destination };
  }

  /**
   * Build with all parameters pre-resolved — no asker prompts. Used by the
   * agent CLI / server where scheme, configuration, and destination come in via
   * flags or workspace state.
   */
  async buildExplicit(options: {
    scheme: string;
    configuration: string;
    destination: Destination;
    xcworkspace: string;
    debug: boolean;
    /**
     * Per-line sink for the xcodebuild process output. Invoked alongside the
     * diagnostics collector so callers (the agent server) can tee the stream
     * into a log file or push events to subscribers.
     */
    onOutputLine?: (line: string) => void;
  }): Promise<void> {
    const destinationRaw = getXcodeBuildDestinationString({
      destination: options.destination,
      config: this.config,
    });
    const sdk = options.destination.platform;

    await this.runSchemeTask({
      name: "Build",
      scheme: options.scheme,
      callback: async (terminal) => {
        await this.buildApp(terminal, {
          scheme: options.scheme,
          sdk: sdk,
          configuration: options.configuration,
          shouldBuild: true,
          shouldClean: false,
          shouldTest: false,
          xcworkspace: options.xcworkspace,
          destinationRaw: destinationRaw,
          debug: options.debug,
          onOutputLine: options.onOutputLine,
        });
      },
    });
  }

  /**
   * Run application on the simulator or device without building
   */
  async runCommand(item: { scheme?: string } | undefined, options: { debug: boolean }) {
    this.progress.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.askDeps, this);

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.askDeps, this, {
        title: "Select scheme to build and run",
        xcworkspace: xcworkspace,
      }));

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.askDeps, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.askDeps, this.destinations, {
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });

    const sdk = destination.platform;

    const schemeSettings = await getSchemeLaunchSettings({ logger: this.logger }, { xcworkspace, scheme });
    const launchArgs = [...schemeSettings.args, ...(this.config.get("build.launchArgs") ?? [])];
    const launchEnv = { ...schemeSettings.env, ...this.config.get("build.launchEnv") };

    await this.runSchemeTask({
      name: "Run",
      scheme: scheme,
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
  async launchCommand(item: { scheme?: string } | undefined, options: { debug: boolean }) {
    this.progress.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.askDeps, this);

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.askDeps, this, {
        title: "Select scheme to build and run",
        xcworkspace: xcworkspace,
      }));

    await generateBuildServerConfigOnBuild(
      { ...this.xcodeCli, lsp: this.lsp },
      {
        scheme: scheme,
        xcworkspace: xcworkspace,
      },
    );

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.askDeps, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.askDeps, this.destinations, {
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });

    const destinationRaw = getXcodeBuildDestinationString({ destination: destination, config: this.config });

    const sdk = destination.platform;

    const schemeSettings = await getSchemeLaunchSettings({ logger: this.logger }, { xcworkspace, scheme });
    const launchArgs = [...schemeSettings.args, ...(this.config.get("build.launchArgs") ?? [])];
    const launchEnv = { ...schemeSettings.env, ...this.config.get("build.launchEnv") };

    await this.runSchemeTask({
      name: options.debug ? "Debug" : "Launch",
      scheme: scheme,
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
    const buildSettings = await getBuildSettingsToLaunch(this.xcodeCli, {
      scheme: options.scheme,
      configuration: options.configuration,
      sdk: "macosx",
      xcworkspace: options.xcworkspace,
    });

    const executablePath = await ensureAppPathExists(buildSettings.executablePath);

    this.state.update("build.lastLaunchedApp", {
      type: "macos",
      appPath: executablePath,
      bundleIdentifier: buildSettings.bundleIdentifier,
    });
    if (options.watchMarker) {
      writeWatchMarkers(terminal);
    }

    this.progress.updateText(`Running "${options.scheme}" on Mac`);
    await terminal.runGroup(async (group) => {
      const logSidecar = new MacOSLogSidecar(
        group,
        { logger: this.logger, config: this.config, cwd: this.workspaceRoot.getPath() },
        {
          bundleId: buildSettings.bundleIdentifier,
          executableName: buildSettings.executableName,
        },
      );
      await logSidecar.spawn();

      const main = new MainExecutable(group, {
        command: executablePath,
        args: options.launchArgs,
        // NSUnbufferedIO is a no-op when stdout is a tty (the v3/node-pty path), but acts as a
        // safety net for the v2 fallback where stdout is a plain pipe and Foundation block-buffers print().
        env: { NSUnbufferedIO: "YES", ...options.launchEnv },
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
    const buildSettings = await getBuildSettingsToLaunch(this.xcodeCli, {
      scheme: options.scheme,
      configuration: options.configuration,
      sdk: options.sdk,
      xcworkspace: options.xcworkspace,
    });
    const appPath = await ensureAppPathExists(buildSettings.appPath);
    const bundlerId = buildSettings.bundleIdentifier;

    // Get simulator with fresh state
    this.progress.updateText(`Searching for simulator "${simulatorId}"`);
    const simulator = await getSimulatorByUdid(this.destinations, {
      udid: simulatorId,
    });

    if (!simulator.isBooted) {
      this.progress.updateText(`Booting simulator "${simulator.name}"`);
      await terminal.execute({
        command: "xcrun",
        args: ["simctl", "boot", simulator.udid],
      });

      this.destinations.refreshSimulators();
    }

    this.progress.updateText("Launching Simulator.app");
    const bringToForeground = this.config.get("build.bringSimulatorToForeground") ?? true;
    const openArgs = bringToForeground ? ["-a", "Simulator"] : ["-g", "-a", "Simulator"];
    await terminal.execute({
      command: "open",
      args: openArgs,
    });

    this.progress.updateText(`Installing "${options.scheme}" on "${simulator.name}"`);
    await terminal.execute({
      command: "xcrun",
      args: ["simctl", "install", simulator.udid, appPath],
    });

    this.state.update("build.lastLaunchedApp", {
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
      // This instructs the app to wait for the debugger before launching so we can attach to early init.
      ...(options.debug ? ["--wait-for-debugger"] : []),
      "--terminate-running-process",
      simulator.udid,
      bundlerId,
      ...options.launchArgs,
    ];

    this.progress.updateText(`Running "${options.scheme}" on "${simulator.name}"`);
    await terminal.runGroup(async (group) => {
      const logSidecar = new SimulatorLogSidecar(
        group,
        { logger: this.logger, config: this.config, cwd: this.workspaceRoot.getPath() },
        {
          simulatorUdid: simulator.udid,
          bundleId: bundlerId,
          executableName: buildSettings.executableName,
        },
      );
      await logSidecar.spawn();

      const main = new MainExecutable(group, {
        command: "xcrun",
        args: launchArgs,
        // simctl strips SIMCTL_CHILD_ and passes the rest to the launched app.
        env: Object.fromEntries(Object.entries(options.launchEnv).map(([k, v]) => [`SIMCTL_CHILD_${k}`, v])),
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
    const buildSettings = await getBuildSettingsToLaunch(this.xcodeCli, {
      scheme: scheme,
      configuration: configuration,
      sdk: option.sdk,
      xcworkspace: option.xcworkspace,
    });

    const targetPath = await ensureAppPathExists(buildSettings.appPath);
    const bundlerId = buildSettings.bundleIdentifier;

    const useDevicectl = destination.supportsDevicectl;
    const deviceId = useDevicectl ? destination.devicectlId : destination.udid;

    if (!deviceId) {
      throw new ExtensionError(`Could not determine device ID for ${destinationName}`);
    }

    this.progress.updateText(`Installing "${scheme}" on "${destinationName}"`);

    if (option.watchMarker) {
      writeWatchMarkers(terminal);
    }

    this.progress.updateText(`Running "${option.scheme}" on "${option.destination.name}"`);

    if (useDevicectl) {
      // Use devicectl for iOS 17+ devices - separate install and launch
      await terminal.execute({
        command: "xcrun",
        args: ["devicectl", "device", "install", "app", "--device", deviceId, targetPath],
      });

      await using jsonOutputPath = await tempFilePath(await this.workspaceRoot.getStoragePath(), {
        prefix: "json",
      });

      this.progress.updateText("Extracting Xcode version");
      const xcodeVersion = await getXcodeVersionInstalled(this.xcodeCli);
      const isConsoleOptionSupported = xcodeVersion.major >= 16;

      this.state.update("build.lastLaunchedApp", {
        type: "device",
        appPath: targetPath,
        appName: buildSettings.appName,
        executableName: buildSettings.executableName,
        bundleIdentifier: bundlerId,
        destinationId: deviceId,
        destinationType: destinationType,
      });

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
      ].filter((arg) => arg !== null);

      this.progress.updateText(`Running "${option.scheme}" on "${option.destination.name}"`);

      if (this.beforeDeviceLaunch) {
        await this.beforeDeviceLaunch();
      }

      await terminal.runGroup(async (group) => {
        // pymobiledevice3 is the only device log backend; toggle the global
        // build.logStreamEnabled to disable. Pymd3Sidecar.spec() returns null and writes
        // a [sweetpad] warning when streaming is disabled, the binary is missing, or the
        // executable name is unknown; pymd3's own stderr (e.g. tunneld not running)
        // surfaces via [pymobiledevice3]. The launch proceeds either way.
        const logSidecar = new Pymd3Sidecar(
          group,
          { logger: this.logger, config: this.config, cwd: this.workspaceRoot.getPath() },
          {
            executableName: buildSettings.executableName,
            enableDebugDylib: buildSettings.enableDebugDylib,
          },
        );
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
      // Use ios-deploy for older devices (iOS < 17). It handles both install and launch
      // in one command with --debug.
      this.logger.debug("Using ios-deploy for older device", {
        deviceId: deviceId,
        osVersion: destination.osVersion,
      });

      const isInstalled = await iosDeploy.isIosDeployInstalled({
        cwd: this.workspaceRoot.getPath(),
        logger: this.logger,
      });
      if (!isInstalled) {
        throw new ExtensionError("ios-deploy is required for iOS < 17. Install it with: brew install ios-deploy");
      }

      this.state.update("build.lastLaunchedApp", {
        type: "device",
        appPath: targetPath,
        appName: buildSettings.appName,
        executableName: buildSettings.executableName,
        bundleIdentifier: bundlerId,
        destinationId: deviceId,
        destinationType: destinationType,
      });

      await iosDeploy.installAndLaunchApp(terminal, {
        storagePath: await this.workspaceRoot.getStoragePath(),
        deviceId: deviceId,
        appPath: targetPath,
        bundleId: bundlerId,
        launchArgs: option.launchArgs,
        launchEnv: option.launchEnv,
        logger: this.logger,
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
      /**
       * Optional tee for every output line. Composed with the diagnostics
       * collector — diagnostics still receive the line; the caller-provided
       * sink runs after, so it sees the same raw text xcodebuild emits.
       */
      onOutputLine?: (line: string) => void;
    },
  ) {
    const useXcbeautify = isXcbeautifyEnabled(this.config) && (await getIsXcbeautifyInstalled(this.xcodeCli));
    const bundlePath = await prepareBundleDir(await this.workspaceRoot.getStoragePath(), options.scheme);
    const derivedDataPath = prepareDerivedDataPath({ config: this.config, cwd: this.workspaceRoot.getPath() });

    const arch = this.config.get("build.arch") || undefined;
    const allowProvisioningUpdates = this.config.get("build.allowProvisioningUpdates") ?? true;

    const additionalArgs: string[] = this.config.get("build.args") || [];

    const env = this.config.get("build.env") || {};

    const workspaceType = detectWorkspaceType(options.xcworkspace);

    const command = new XcodeCommandBuilder(this.xcodeCli);
    if (arch) {
      command.addBuildSettings("ARCHS", arch);
      command.addBuildSettings("VALID_ARCHS", arch);
      command.addBuildSettings("ONLY_ACTIVE_ARCH", "NO");
    }

    if (options.debug) {
      // GCC_GENERATE_DEBUGGING_SYMBOLS=YES — LLDB needs DWARF symbols to match source lines.
      // Xcode debug builds set this by default, but we force it so sweetpad: debugging-launch
      // works regardless of the scheme's Build Configuration.
      command.addBuildSettings("GCC_GENERATE_DEBUGGING_SYMBOLS", "YES");
      // ONLY_ACTIVE_ARCH=YES — compile for just the current device's slice (no fat binary)
      // so debug builds finish faster.
      command.addBuildSettings("ONLY_ACTIVE_ARCH", "YES");
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

    await generateBuildServerConfigOnBuild(
      { ...this.xcodeCli, lsp: this.lsp },
      {
        scheme: options.scheme,
        xcworkspace: options.xcworkspace,
      },
    );

    let cwd: string;
    if (workspaceType === "spm") {
      cwd = getSwiftPMDirectory(options.xcworkspace);
    } else if (workspaceType === "xcode") {
      cwd = this.workspaceRoot.getPath();
    } else {
      assertUnreachable(workspaceType);
    }

    const diagnostics = this.diagnostics.beginBuild({ mode: useXcbeautify ? "xcbeautify" : "xcodebuild" });
    const externalSink = options.onOutputLine;
    try {
      await terminal.execute({
        command: commandParts[0],
        args: commandParts.slice(1),
        pipes: pipes,
        env: env,
        cwd: cwd,
        closeStdin: true,
        onOutputLine: async ({ value }) => {
          diagnostics.recordLine(value);
          externalSink?.(value);
        },
      });
    } finally {
      diagnostics.flush();
    }

    await this.lsp.refresh();
  }

  async cleanCommand(item: { scheme?: string } | undefined) {
    this.progress.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.askDeps, this);

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.askDeps, this, { title: "Select scheme to clean", xcworkspace: xcworkspace }));

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.askDeps, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.askDeps, this.destinations, {
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });
    const destinationRaw = getXcodeBuildDestinationString({ destination: destination, config: this.config });

    const sdk = destination.platform;

    await this.runSchemeTask({
      name: "Clean",
      scheme: scheme,
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

  async testCommand(item: { scheme?: string } | undefined) {
    this.progress.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.askDeps, this);

    this.progress.updateText("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(this.askDeps, this, { title: "Select scheme to test", xcworkspace: xcworkspace }));

    this.progress.updateText("Searching for configuration");
    const configuration = await askConfiguration(this.askDeps, this, { xcworkspace: xcworkspace });

    this.progress.updateText("Searching for destination");
    const destination = await askDestinationToRunOn(this.askDeps, this.destinations, {
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });
    const destinationRaw = getXcodeBuildDestinationString({ destination: destination, config: this.config });

    const sdk = destination.platform;

    await this.runSchemeTask({
      name: "Test",
      scheme: scheme,
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
      callback: async (terminal) => {
        const workspaceType = detectWorkspaceType(options.xcworkspace);
        if (workspaceType === "spm") {
          const packageDir = getSwiftPMDirectory(options.xcworkspace);
          await terminal.execute({
            command: getSwiftCommand(this.xcodeCli),
            args: ["package", "resolve"],
            cwd: packageDir,
          });
        } else if (workspaceType === "xcode") {
          await terminal.execute({
            command: getXcodeBuildCommand(this.xcodeCli),
            args: ["-resolvePackageDependencies", "-scheme", options.scheme, "-workspace", options.xcworkspace],
            closeStdin: true,
          });
        } else {
          assertUnreachable(workspaceType);
        }
      },
    });
  }

  async stopSchemeCommand(item: { scheme?: string } | undefined): Promise<void> {
    const scheme = item?.scheme;
    if (!scheme) return;

    this.taskRunner.stopMatching({ lock: "sweetpad.build", metadata: { scheme: scheme } });
    this.stopSchemeBuild(scheme);
  }
}
