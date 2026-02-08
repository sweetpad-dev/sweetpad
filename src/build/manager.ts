import events from "node:events";
import * as path from "node:path";
import * as vscode from "vscode";
import {
  type XcodeScheme,
  generateBuildServerConfig,
  getBasicProjectInfo,
  getBuildSettingsToLaunch,
  getIsXcbeautifyInstalled,
  getIsXcodeBuildServerInstalled,
  getSchemes,
  getSwiftCommand,
  getXcodeBuildCommand,
  getXcodeVersionInstalled,
} from "../common/cli/scripts";
import { BaseExecutionScope, type ExtensionContext } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { ExtensionError } from "../common/errors";
import { isFileExists, readJsonFile, tempFilePath } from "../common/files";
import { commonLogger } from "../common/logger";
import { type Command, type TaskTerminal, runTask } from "../common/tasks";
import { assertUnreachable } from "../common/types";
import * as iosDeploy from "../common/xcode/ios-deploy";
import { getLogStreamManager } from "../debugger/log-stream";
import type { DeviceDestination } from "../devices/types";
import type { SimulatorDestination } from "../simulators/types";
import { getSimulatorByUdid } from "../simulators/utils";
import { DEFAULT_BUILD_PROBLEM_MATCHERS } from "./constants";
import type { BuildTreeItem } from "./tree";
import {
  XcodeCommandBuilder,
  askConfiguration,
  askDestinationToRunOn,
  askSchemeForBuild,
  askXcodeWorkspacePath,
  detectWorkspaceType,
  ensureAppPathExists,
  generateBuildServerConfigOnBuild,
  getCurrentXcodeWorkspacePath,
  getSwiftPMDirectory,
  getWorkspacePath,
  getXcodeBuildDestinationString,
  isXcbeautifyEnabled,
  prepareBundleDir,
  prepareDerivedDataPath,
  restartSwiftLSP,
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

export class BuildManager {
  private cache: XcodeScheme[] | undefined = undefined;
  private emitter = new events.EventEmitter<IEventMap>();
  public _context: ExtensionContext | undefined = undefined;
  private runningSchemes: Set<string> = new Set();

  constructor() {
    this.on("defaultSchemeForBuildUpdated", (scheme: string | undefined) => {
      void this.generateXcodeBuildServerSettingsOnSchemeChange({
        scheme: scheme,
      });
    });
  }

  on<K extends IEventKey>(event: K, listener: (...args: IEventMap[K]) => void): void {
    this.emitter.on(event, listener as any); // todo: fix this any
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

  set context(context: ExtensionContext) {
    this._context = context;
  }

  get context(): ExtensionContext {
    if (!this._context) {
      throw new Error("Context is not set");
    }
    return this._context;
  }

  async refreshSchemes(): Promise<XcodeScheme[]> {
    const scope = new BaseExecutionScope();
    return await this.context.startExecutionScope(scope, async () => {
      this.context.updateProgressStatus("Refreshing Xcode schemes");

      this.emitter.emit("refreshSchemesStarted");
      try {
        getBasicProjectInfo.clearCache();

        const xcworkspace = getCurrentXcodeWorkspacePath(this.context);

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
    return this.context.getWorkspaceState("build.xcodeScheme");
  }

  getDefaultSchemeForTesting(): string | undefined {
    return this.context.getWorkspaceState("testing.xcodeScheme");
  }

  setDefaultSchemeForBuild(scheme: string | undefined): void {
    this.context.updateWorkspaceState("build.xcodeScheme", scheme);
    this.emitter.emit("defaultSchemeForBuildUpdated", scheme);
  }

  setDefaultSchemeForTesting(scheme: string | undefined): void {
    this.context.updateWorkspaceState("testing.xcodeScheme", scheme);
    this.emitter.emit("defaultSchemeForTestingUpdated", scheme);
  }

  getDefaultConfigurationForBuild(): string | undefined {
    return this.context.getWorkspaceState("build.xcodeConfiguration");
  }

  getDefaultConfigurationForTesting(): string | undefined {
    return this.context.getWorkspaceState("testing.xcodeConfiguration");
  }

  setDefaultConfigurationForBuild(configuration: string | undefined): void {
    this.context.updateWorkspaceState("build.xcodeConfiguration", configuration);
  }

  setDefaultConfigurationForTesting(configuration: string | undefined): void {
    this.context.updateWorkspaceState("testing.xcodeConfiguration", configuration);
  }

  /**
   * Every time the scheme changes, we need to rebuild the buildServer.json file
   * for providing the correct build settings to the LSP server.
   */
  async generateXcodeBuildServerSettingsOnSchemeChange(options: {
    scheme: string | undefined;
  }): Promise<void> {
    if (!options.scheme) {
      return;
    }

    const isEnabled = getWorkspaceConfig("xcodebuildserver.autogenerate") ?? true;
    if (!isEnabled) {
      return;
    }

    const buildServerJsonPath = path.join(getWorkspacePath(), "buildServer.json");
    const isBuildServerJsonExists = await isFileExists(buildServerJsonPath);
    if (!isBuildServerJsonExists) {
      return;
    }

    const isServerInstalled = await getIsXcodeBuildServerInstalled();
    if (!isServerInstalled) {
      return;
    }

    const xcworkspace = await askXcodeWorkspacePath(this.context);
    await generateBuildServerConfig({
      xcworkspace: xcworkspace,
      scheme: options.scheme,
    });
    await restartSwiftLSP();

    const isShown = this.context.getWorkspaceState("build.xcodeBuildServerAutogenreateInfoShown") ?? false;
    if (!isShown) {
      this.context.updateWorkspaceState("build.xcodeBuildServerAutogenreateInfoShown", true);
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

    const schemeNames = this.cache.map((scheme) => scheme.name);
    const currentBuildScheme = this.getDefaultSchemeForBuild();
    if (currentBuildScheme && !schemeNames.includes(currentBuildScheme)) {
      this.setDefaultSchemeForBuild(undefined);
    }

    const currentTestingScheme = this.getDefaultSchemeForTesting();
    if (currentTestingScheme && !schemeNames.includes(currentTestingScheme)) {
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
      await runTask(this.context, {
        name: options.name,
        lock: "sweetpad.build",
        terminateLocked: true,
        problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
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
  async buildCommand(item: BuildTreeItem | undefined, options: { debug: boolean }) {
    const context = this.context;

    context.updateProgressStatus("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(context);

    context.updateProgressStatus("Searching for scheme");
    const scheme =
      item?.scheme ?? (await askSchemeForBuild(context, { title: "Select scheme to build", xcworkspace: xcworkspace }));

    await generateBuildServerConfigOnBuild({
      scheme: scheme,
      xcworkspace: xcworkspace,
    });

    context.updateProgressStatus("Searching for configuration");
    const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

    context.updateProgressStatus("Searching for destination");
    const destination = await askDestinationToRunOn(context, {
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
    const context = this.context;
    context.updateProgressStatus("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(context);

    context.updateProgressStatus("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(context, { title: "Select scheme to build and run", xcworkspace: xcworkspace }));

    context.updateProgressStatus("Searching for configuration");
    const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

    context.updateProgressStatus("Searching for destination");
    const destination = await askDestinationToRunOn(context, {
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });

    const sdk = destination.platform;

    const launchArgs = getWorkspaceConfig("build.launchArgs") ?? [];
    const launchEnv = getWorkspaceConfig("build.launchEnv") ?? {};

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
  async launchCommand(item: BuildTreeItem | undefined, options: { debug: boolean }) {
    const context = this.context;
    context.updateProgressStatus("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(context);

    context.updateProgressStatus("Searching for scheme");
    const scheme =
      item?.scheme ??
      (await askSchemeForBuild(context, { title: "Select scheme to build and run", xcworkspace: xcworkspace }));

    await generateBuildServerConfigOnBuild({
      scheme: scheme,
      xcworkspace: xcworkspace,
    });

    context.updateProgressStatus("Searching for configuration");
    const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

    context.updateProgressStatus("Searching for destination");
    const destination = await askDestinationToRunOn(context, {
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });

    const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

    const sdk = destination.platform;

    const launchArgs = getWorkspaceConfig("build.launchArgs") ?? [];
    const launchEnv = getWorkspaceConfig("build.launchEnv") ?? {};

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
    const context = this.context;

    context.updateProgressStatus("Extracting build settings");
    const buildSettings = await getBuildSettingsToLaunch({
      scheme: options.scheme,
      configuration: options.configuration,
      sdk: "macosx",
      xcworkspace: options.xcworkspace,
    });

    const executablePath = await ensureAppPathExists(buildSettings.executablePath);

    context.updateWorkspaceState("build.lastLaunchedApp", {
      type: "macos",
      appPath: executablePath,
      bundleIdentifier: buildSettings.bundleIdentifier,
    });
    if (options.watchMarker) {
      writeWatchMarkers(terminal);
    }

    // Prepare log stream output channel for stdout/stderr capture
    const logStreamManager = getLogStreamManager(context);
    logStreamManager.prepareForLaunch(buildSettings.bundleIdentifier);

    context.updateProgressStatus(`Running "${options.scheme}" on Mac`);
    await terminal.execute({
      command: executablePath,
      env: options.launchEnv,
      args: options.launchArgs,
      // Forward stdout/stderr to the log stream output channel
      onOutputLine: async (data) => {
        logStreamManager.appendOutput(data.value, data.type);
      },
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
    const context = this.context;
    const simulatorId = options.destination.udid;

    context.updateProgressStatus("Extracting build settings");
    const buildSettings = await getBuildSettingsToLaunch({
      scheme: options.scheme,
      configuration: options.configuration,
      sdk: options.sdk,
      xcworkspace: options.xcworkspace,
    });
    const appPath = await ensureAppPathExists(buildSettings.appPath);
    const bundlerId = buildSettings.bundleIdentifier;

    // Get simulator with fresh state
    context.updateProgressStatus(`Searching for simulator "${simulatorId}"`);
    const simulator = await getSimulatorByUdid(context, {
      udid: simulatorId,
    });

    // Boot device
    if (!simulator.isBooted) {
      context.updateProgressStatus(`Booting simulator "${simulator.name}"`);
      await terminal.execute({
        command: "xcrun",
        args: ["simctl", "boot", simulator.udid],
      });

      // Refresh list of simulators after we start new simulator
      context.destinationsManager.refreshSimulators();
    }

    // Open simulator
    context.updateProgressStatus("Launching Simulator.app");
    const bringToForeground = getWorkspaceConfig("build.bringSimulatorToForeground") ?? true;
    const openArgs = bringToForeground ? ["-a", "Simulator"] : ["-g", "-a", "Simulator"];
    await terminal.execute({
      command: "open",
      args: openArgs,
    });

    // Install app
    context.updateProgressStatus(`Installing "${options.scheme}" on "${simulator.name}"`);
    await terminal.execute({
      command: "xcrun",
      args: ["simctl", "install", simulator.udid, appPath],
    });

    context.updateWorkspaceState("build.lastLaunchedApp", {
      type: "simulator",
      appPath: appPath,
      bundleIdentifier: bundlerId,
      simulatorUdid: simulator.udid,
    });
    if (options.watchMarker) {
      writeWatchMarkers(terminal);
    }

    // Prepare log stream output channel for stdout/stderr capture
    const logStreamManager = getLogStreamManager(context);
    logStreamManager.prepareForLaunch(bundlerId);

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
    context.updateProgressStatus(`Running "${options.scheme}" on "${simulator.name}"`);
    await terminal.execute({
      command: "xcrun",
      args: launchArgs,
      // should be prefixed with `SIMCTL_CHILD_` to pass to the child process
      env: Object.fromEntries(Object.entries(options.launchEnv).map(([key, value]) => [`SIMCTL_CHILD_${key}`, value])),
      // Forward stdout/stderr to the log stream output channel
      onOutputLine: async (data) => {
        logStreamManager.appendOutput(data.value, data.type);
      },
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
    const context = this.context;
    const { scheme, configuration, destination } = option;
    const { type: destinationType, name: destinationName } = destination;

    context.updateProgressStatus("Extracting build settings");
    const buildSettings = await getBuildSettingsToLaunch({
      scheme: scheme,
      configuration: configuration,
      sdk: option.sdk,
      xcworkspace: option.xcworkspace,
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
    context.updateProgressStatus(`Installing "${scheme}" on "${destinationName}"`);

    context.updateWorkspaceState("build.lastLaunchedApp", {
      type: "device",
      appPath: targetPath,
      appName: buildSettings.appName,
      bundleIdentifier: bundlerId,
      destinationId: deviceId,
      destinationType: destinationType,
    });

    if (option.watchMarker) {
      writeWatchMarkers(terminal);
    }

    // Prepare log stream output channel for stdout/stderr capture
    const logStreamManager = getLogStreamManager(context);
    logStreamManager.prepareForLaunch(bundlerId);

    // Launch app on device
    context.updateProgressStatus(`Running "${option.scheme}" on "${option.destination.name}"`);

    if (useDevicectl) {
      // Use devicectl for iOS 17+ devices - separate install and launch
      await terminal.execute({
        command: "xcrun",
        args: ["devicectl", "device", "install", "app", "--device", deviceId, targetPath],
      });

      await using jsonOutputPath = await tempFilePath(context, {
        prefix: "json",
      });

      context.updateProgressStatus("Extracting Xcode version");
      const xcodeVersion = await getXcodeVersionInstalled();
      const isConsoleOptionSupported = xcodeVersion.major >= 16;

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

      await terminal.execute({
        command: "xcrun",
        args: launchArgs,
        // Should be prefixed with `DEVICECTL_CHILD_` to pass to the child process
        env: Object.fromEntries(
          Object.entries(option.launchEnv).map(([key, value]) => [`DEVICECTL_CHILD_${key}`, value]),
        ),
        // Forward stdout/stderr to the log stream output channel
        onOutputLine: async (data) => {
          logStreamManager.appendOutput(data.value, data.type);
        },
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

      if (option.watchMarker) {
        writeWatchMarkers(terminal);
      }

      await iosDeploy.installAndLaunchApp(context, terminal, {
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
    const context = this.context;

    const useXcbeautify = isXcbeautifyEnabled() && (await getIsXcbeautifyInstalled());
    const bundlePath = await prepareBundleDir(context, options.scheme);
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
      context.updateProgressStatus(`Cleaning "${options.scheme}"`);
    } else if (options.shouldBuild) {
      context.updateProgressStatus(`Building "${options.scheme}"`);
    } else if (options.shouldTest) {
      context.updateProgressStatus(`Building "${options.scheme}"`);
    }

    await generateBuildServerConfigOnBuild({
      scheme: options.scheme,
      xcworkspace: options.xcworkspace,
    });

    let cwd: string;
    if (workspaceType === "spm") {
      cwd = getSwiftPMDirectory(options.xcworkspace);
    } else if (workspaceType === "xcode") {
      cwd = getWorkspacePath();
    } else {
      assertUnreachable(workspaceType);
    }

    await terminal.execute({
      command: commandParts[0],
      args: commandParts.slice(1),
      pipes: pipes,
      env: env,
      cwd: cwd,
    });

    await restartSwiftLSP();
  }

  async cleanCommand(item: BuildTreeItem | undefined) {
    const context = this.context;
    context.updateProgressStatus("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(context);

    context.updateProgressStatus("Searching for scheme");
    const scheme =
      item?.scheme ?? (await askSchemeForBuild(context, { title: "Select scheme to clean", xcworkspace: xcworkspace }));

    context.updateProgressStatus("Searching for configuration");
    const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

    context.updateProgressStatus("Searching for destination");
    const destination = await askDestinationToRunOn(context, {
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
    const context = this.context;

    context.updateProgressStatus("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(context);

    context.updateProgressStatus("Searching for scheme");
    const scheme =
      item?.scheme ?? (await askSchemeForBuild(context, { title: "Select scheme to test", xcworkspace: xcworkspace }));

    context.updateProgressStatus("Searching for configuration");
    const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

    context.updateProgressStatus("Searching for destination");
    const destination = await askDestinationToRunOn(context, {
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
    const context = this.context;
    context.updateProgressStatus("Resolving dependencies");

    await this.runSchemeTask({
      name: "Resolve Dependencies",
      scheme: options.scheme,
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
          });
        } else {
          assertUnreachable(workspaceType);
        }
      },
    });
  }

  async stopSchemeCommand(item: BuildTreeItem | undefined): Promise<void> {
    const context = this.context;

    const scheme = item?.scheme;
    if (!scheme) return;

    const tasks = vscode.tasks.taskExecutions.filter(
      ({ task }) => task.definition.lockId === "sweetpad.build" && task.definition.metadata?.scheme === scheme,
    );
    for (const task of tasks) {
      task.terminate();
    }
    // Ensure the scheme is marked as stopped in the manager
    context.buildManager.stopSchemeBuild(scheme);
  }
}
