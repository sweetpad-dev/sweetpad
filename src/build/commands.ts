import path from "node:path";
import * as vscode from "vscode";
import type { BuildTreeItem, WorkspaceGroupTreeItem } from "./tree";

import { showConfigurationPicker, showYesNoQuestion } from "../common/askers";
import {
  type XcodeScheme,
  generateBuildServerConfig,
  getBuildConfigurations,
  getBuildSettingsToAskDestination,
  getBuildSettingsToLaunch,
  getIsXcbeautifyInstalled,
  getIsXcodeBuildServerInstalled,
  getXcodeVersionInstalled,
} from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { getWorkspaceConfig, updateWorkspaceConfig } from "../common/config";
import { ExecBaseError, ExtensionError } from "../common/errors";
import { exec } from "../common/exec";
import { getWorkspaceRelativePath, isFileExists, readJsonFile, removeDirectory, tempFilePath } from "../common/files";
import { readdir } from "node:fs/promises";
import { commonLogger } from "../common/logger";

import { type Command, type TaskTerminal, runTask } from "../common/tasks";
import { assertUnreachable } from "../common/types";
import type { Destination } from "../destination/types";
import type { DeviceDestination } from "../devices/types";
import type { SimulatorDestination } from "../simulators/types";
import { getSimulatorByUdid } from "../simulators/utils";
import { DEFAULT_BUILD_PROBLEM_MATCHERS } from "./constants";
import {
  askConfiguration,
  askDestinationToRunOn,
  askSchemeForBuild,
  askXcodeWorkspacePath,
  detectXcodeWorkspacesPaths,
  getCurrentXcodeWorkspacePath,
  getWorkspacePath,
  prepareBundleDir,
  prepareDerivedDataPath,
  prepareStoragePath,
  restartSwiftLSP,
  selectXcodeWorkspace,
} from "./utils";

function writeWatchMarkers(terminal: TaskTerminal) {
  terminal.write("🍭 SweetPad: watch marker (start)\n");
  terminal.write("🍩 SweetPad: watch marker (end)\n\n");
}

async function ensureAppPathExists(appPath: string | undefined): Promise<string> {
  if (!appPath) {
    throw new ExtensionError("App path is empty. Something went wrong.");
  }

  const isExists = await isFileExists(appPath);
  if (!isExists) {
    throw new ExtensionError(`App path does not exist. Have you built the app? Path: ${appPath}`);
  }
  return appPath;
}

export async function runOnMac(
  context: ExtensionContext,
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
  context.updateProgressStatus("Extracting build settings");
  vscode.window.showInformationMessage(`Running application on macOS...`);
  terminal.write("Preparing to execute runOnMac command...\n");
  
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
  });
  if (options.watchMarker) {
    writeWatchMarkers(terminal);
  }

  context.updateProgressStatus(`Running "${options.scheme}" on Mac`);
  await terminal.execute({
    command: executablePath,
    env: options.launchEnv,
    args: options.launchArgs,
  });
}

export async function runOniOSSimulator(
  context: ExtensionContext,
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

  context.updateProgressStatus("Extracting build settings");
  vscode.window.showInformationMessage(`Running application on iOS Simulator...`);
  terminal.write("Preparing to execute runOniOSSimulator command...\n");
  
  const buildSettings = await getBuildSettingsToLaunch({
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: options.sdk,
    xcworkspace: options.xcworkspace,
  });
  const appPath = await ensureAppPathExists(buildSettings.appPath);
  const bundlerId = buildSettings.bundleIdentifier;

  // Open simulator
  context.updateProgressStatus("Launching Simulator.app");
  await terminal.execute({
    command: "open",
    args: ["-g", "-a", "Simulator"],
  });

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

  // Install app
  context.updateProgressStatus(`Installing "${options.scheme}" on "${simulator.name}"`);
  await terminal.execute({
    command: "xcrun",
    args: ["simctl", "install", simulator.udid, appPath],
  });

  context.updateWorkspaceState("build.lastLaunchedApp", {
    type: "simulator",
    appPath: appPath,
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
  context.updateProgressStatus(`Running "${options.scheme}" on "${simulator.name}"`);
  await terminal.execute({
    command: "xcrun",
    args: launchArgs,
    // should be prefixed with `SIMCTL_CHILD_` to pass to the child process
    env: Object.fromEntries(Object.entries(options.launchEnv).map(([key, value]) => [`SIMCTL_CHILD_${key}`, value])),
  });
}

export async function runOniOSDevice(
  context: ExtensionContext,
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
  const { udid: deviceId, type: destinationType, name: destinationName } = destination;

  vscode.window.showInformationMessage(`Running application on iOS device...`);
  terminal.write("Preparing to execute runOniOSDevice command...\n");

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToLaunch({
    scheme: scheme,
    configuration: configuration,
    sdk: option.sdk,
    xcworkspace: option.xcworkspace,
  });

  const targetPath = await ensureAppPathExists(buildSettings.appPath);
  const bundlerId = buildSettings.bundleIdentifier;

  // Install app on device
  context.updateProgressStatus(`Installing "${scheme}" on "${destinationName}"`);
  try {
    await terminal.execute({
      command: "xcrun",
      args: ["devicectl", "device", "install", "app", "--device", deviceId, targetPath],
    });
  } catch (error) {
    // Check for passcode protection error
    if (error instanceof Error && isPasscodeProtectionError(error)) {
      const helpfulMessage = `🔒 Device is passcode protected or not trusted.\n\n` +
        `Please follow these steps:\n` +
        `1. 🔓 Unlock your iOS device (${destinationName})\n` +
        `2. 🔌 If connected via USB, disconnect and reconnect the device\n` +
        `3. 📱 When prompted on the device, tap "Trust This Computer"\n` +
        `4. 🔑 Enter your device passcode if requested\n` +
        `5. 🔄 Try running the command again\n\n` +
        `💡 Tip: Keep your device unlocked during app installation and launch.`;
      
      terminal.write(helpfulMessage, { newLine: true, color: "yellow" });
      throw new ExtensionError("Device passcode protection prevents app installation. Please unlock device and trust this computer.");
    }
    throw error; // Re-throw other errors
  }

  context.updateWorkspaceState("build.lastLaunchedApp", {
    type: "device",
    appPath: targetPath,
    appName: buildSettings.appName,
    destinationId: deviceId,
    destinationType: destinationType,
  });

  await using jsonOuputPath = await tempFilePath(context, {
    prefix: "json",
  });

  context.updateProgressStatus("Extracting Xcode version");
  const xcodeVersion = await getXcodeVersionInstalled();
  const isConsoleOptionSupported = xcodeVersion.major >= 16;

  if (option.watchMarker) {
    writeWatchMarkers(terminal);
  }

  // Prepare the launch arguments
  const launchArgs = [
    "devicectl",
    "device",
    "process",
    "launch",
    // Attaches the application to the console and waits for it to exit
    isConsoleOptionSupported ? "--console" : null,
    "--json-output",
    jsonOuputPath.path,
    // Terminates any already-running instances of the app prior to launch. Not supported on all platforms.
    "--terminate-existing",
    "--device",
    deviceId,
    bundlerId,
    ...option.launchArgs,
  ].filter((arg) => arg !== null); // Filter out null arguments

  // Launch app on device
  context.updateProgressStatus(`Running "${option.scheme}" on "${option.destination.name}"`);
  try {
    await terminal.execute({
      command: "xcrun",
      args: launchArgs,
      // Should be prefixed with `DEVICECTL_CHILD_` to pass to the child process
      env: Object.fromEntries(Object.entries(option.launchEnv).map(([key, value]) => [`DEVICECTL_CHILD_${key}`, value])),
    });
  } catch (error) {
    // Check for passcode protection error during launch
    if (error instanceof Error && isPasscodeProtectionError(error)) {
      const helpfulMessage = `🔒 Device is passcode protected during app launch.\n\n` +
        `Please:\n` +
        `1. 🔓 Ensure your iOS device (${destinationName}) is unlocked\n` +
        `2. 🔄 Try running the command again\n\n` +
        `💡 Tip: Keep your device unlocked during app launch.`;
      
      terminal.write(helpfulMessage, { newLine: true, color: "yellow" });
      throw new ExtensionError("Device passcode protection prevents app launch. Please unlock device.");
    }
    throw error; // Re-throw other errors
  }

  let jsonOutput: any;
  try {
    jsonOutput = await readJsonFile(jsonOuputPath.path);
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
}

/**
 * Check if an error is related to passcode protection on iOS devices
 */
function isPasscodeProtectionError(error: Error): boolean {
  const errorMessage = error.message || "";
  const errorString = error.toString();
  
  // Check for specific error patterns
  const passcodeProtectionPatterns = [
    "The device is passcode protected",
    "DTDKRemoteDeviceConnection: Failed to start remote service",
    "Code=811",
    "Code=-402653158",
    "MobileDeviceErrorCode=(0xE800001A)",
    "com.apple.mobile.notification_proxy",
    "passcode protected"
  ];
  
  return passcodeProtectionPatterns.some(pattern => 
    errorMessage.includes(pattern) || errorString.includes(pattern)
  );
}

/**
 * Handle passcode protection error with helpful user guidance
 */
function handlePasscodeProtectionError(terminal: TaskTerminal, destinationRaw: string): void {
  // Check if this is a simulator destination
  const isSimulatorBuild = destinationRaw.includes("Simulator");
  
  // Extract device name from destination string if possible
  const deviceMatch = destinationRaw.match(/id=([^,]+)/);
  const deviceInfo = deviceMatch ? ` (${deviceMatch[1]})` : "";
  
  let helpfulMessage: string;
  
  if (isSimulatorBuild) {
    helpfulMessage = `🔒 Device passcode protection error during simulator build${deviceInfo}.\n\n` +
      `This can happen when Xcode tries to communicate with connected devices even when building for simulator.\n\n` +
      `Quick solutions:\n` +
      `1. 🔓 Unlock any connected iOS devices\n` +
      `2. 🔌 Disconnect all iOS devices temporarily\n` +
      `3. 📱 If you need devices connected, trust them on each device\n` +
      `4. ⚙️  Consider disabling automatic device detection in Xcode preferences\n` +
      `5. 🔧 Enable 'sweetpad.build.skipDeviceConnectionForSimulator' in VS Code settings\n` +
      `6. 🔄 Try building again\n\n` +
      `💡 Tip: Xcode sometimes tries to connect to devices for background services even during simulator builds.\n` +
      `💡 Alternative: Use 'xcodebuild -destination "platform=iOS Simulator,name=iPhone 15"' to avoid UDID-based selection.\n` +
      `💡 Configuration: The new setting will add build flags to reduce device communication during simulator builds.`;
  } else {
    helpfulMessage = `🔒 Device is passcode protected or not trusted${deviceInfo}.\n\n` +
      `Please follow these steps:\n` +
      `1. 🔓 Unlock your iOS device\n` +
      `2. 🔌 If connected via USB, disconnect and reconnect the device\n` +
      `3. 📱 When prompted on the device, tap "Trust This Computer"\n` +
      `4. 🔑 Enter your device passcode if requested\n` +
      `5. 🔄 Try running the command again\n\n` +
      `💡 Tip: Keep your device unlocked during build and deployment.\n` +
      `💡 For wireless debugging, ensure both devices are on the same network.`;
  }
  
  terminal.write(helpfulMessage, { newLine: true, color: "yellow" });
}

export function isXcbeautifyEnabled() {
  return getWorkspaceConfig("build.xcbeautifyEnabled") ?? true;
}

/**
 * Prepare and return destination string for xcodebuild command.
 *
 * WARN: Do not use result of this function to anything else than xcodebuild command.
 */
export function getXcodeBuildDestinationString(options: { destination: Destination }): string {
  const destination = options.destination;

  if (destination.type === "iOSSimulator") {
    // Specify architecture to avoid ambiguity warnings and reduce device communication attempts
    // Use arm64 for Apple Silicon Macs (M1/M2/M3) for better performance, fallback to x86_64
    const arch = process.arch === "arm64" ? "arm64" : "x86_64";
    return `platform=iOS Simulator,arch=${arch},id=${destination.udid}`;
  }
  if (destination.type === "watchOSSimulator") {
    // watchOS simulators typically use the host architecture
    const arch = process.arch === "arm64" ? "arm64" : "x86_64";
    return `platform=watchOS Simulator,arch=${arch},id=${destination.udid}`;
  }
  if (destination.type === "tvOSSimulator") {
    // tvOS simulators use arm64 on Apple Silicon, x86_64 on Intel
    const arch = process.arch === "arm64" ? "arm64" : "x86_64";
    return `platform=tvOS Simulator,arch=${arch},id=${destination.udid}`;
  }
  if (destination.type === "visionOSSimulator") {
    // visionOS simulators use arm64 architecture
    return `platform=visionOS Simulator,arch=arm64,id=${destination.udid}`;
  }
  if (destination.type === "macOS") {
    // note: without arch, xcodebuild will show warning like this:
    // --- xcodebuild: WARNING: Using the first of multiple matching destinations:
    // { platform:macOS, arch:arm64, id:00008103-000109910EC3001E, name:My Mac }
    // { platform:macOS, arch:x86_64, id:00008103-000109910EC3001E, name:My Mac }
    return `platform=macOS,arch=${destination.arch}`;
  }
  if (destination.type === "iOSDevice") {
    return `platform=iOS,id=${destination.udid}`;
  }
  if (destination.type === "watchOSDevice") {
    return `platform=watchOS,id=${destination.udid}`;
  }
  if (destination.type === "tvOSDevice") {
    return `platform=tvOS,id=${destination.udid}`;
  }
  if (destination.type === "visionOSDevice") {
    return `platform=visionOS,id=${destination.udid}`;
  }
  return assertUnreachable(destination);
}

class XcodeCommandBuilder {
  NO_VALUE = "__NO_VALUE__";

  private xcodebuild = "xcodebuild";
  private parameters: {
    arg: string;
    value: string | "__NO_VALUE__";
  }[] = [];

  private buildSettings: { key: string; value: string }[] = [];
  private actions: string[] = [];

  addBuildSettings(key: string, value: string) {
    this.buildSettings.push({
      key: key,
      value: value,
    });
  }

  addOption(flag: string) {
    this.parameters.push({
      arg: flag,
      value: this.NO_VALUE,
    });
  }

  addParameters(arg: string, value: string) {
    this.parameters.push({
      arg: arg,
      value: value,
    });
  }

  addAction(action: string) {
    this.actions.push(action);
  }

  addAdditionalArgs(args: string[]) {
    for (const current of args) {
      if (current.includes("=")) {
        const [arg, value] = current.split("=");
        this.buildSettings.push({
          key: arg,
          value: value,
        });
      } else if (["clean", "build", "test"].includes(current)) {
        this.actions.push(current);
      } else {
        commonLogger.warn("Unknown argument", {
          argument: current,
          args: args,
        });
      }
    }

    // Remove duplicates, with higher priority for the last occurrence
    const seenParameters = new Set<string>();
    this.parameters = this.parameters
      .slice()
      .reverse()
      .filter((param) => {
        if (seenParameters.has(param.arg)) {
          return false;
        }
        seenParameters.add(param.arg);
        return true;
      })
      .reverse();

    // Remove duplicates, with higher priority for the last occurrence
    const seenActions = new Set<string>();
    this.actions = this.actions.filter((action) => {
      if (seenActions.has(action)) {
        return false;
      }
      seenActions.add(action);
      return true;
    });

    // Remove duplicates, with higher priority for the last occurrence
    const seenSettings = new Set<string>();
    this.buildSettings = this.buildSettings
      .slice()
      .reverse()
      .filter((setting) => {
        if (seenSettings.has(setting.key)) {
          return false;
        }
        seenSettings.add(setting.key);
        return true;
      })
      .reverse();
  }

  build(): string[] {
    const commandParts = [this.xcodebuild];

    for (const { key, value } of this.buildSettings) {
      commandParts.push(`${key}=${value}`);
    }

    for (const { arg, value } of this.parameters) {
      commandParts.push(arg);
      if (value !== this.NO_VALUE) {
        commandParts.push(value);
      }
    }

    for (const action of this.actions) {
      commandParts.push(action);
    }
    return commandParts;
  }
}

export async function buildApp(
  context: ExtensionContext,
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
  vscode.window.showInformationMessage(`Building app for scheme: ${options.scheme}...`);
  terminal.write("Preparing to execute buildApp command...\n");
  const useXcbeatify = isXcbeautifyEnabled() && (await getIsXcbeautifyInstalled());
  const bundlePath = await prepareBundleDir(context, options.scheme);
  const derivedDataPath = prepareDerivedDataPath();

  const arch = getWorkspaceConfig("build.arch") || undefined;
  const allowProvisioningUpdates = getWorkspaceConfig("build.allowProvisioningUpdates") ?? true;

  // ex: ["-arg1", "value1", "-arg2", "value2", "-arg3", "-arg4", "value4"]
  const additionalArgs: string[] = getWorkspaceConfig("build.args") || [];

  // ex: { "ARG1": "value1", "ARG2": null, "ARG3": "value3" }
  const env = getWorkspaceConfig("build.env") || {};

  // Check if this is an SPM project
  const isSPMProject = options.xcworkspace.endsWith("Package.swift");
  
  // Get testing framework preference
  const testingFramework = getWorkspaceConfig("testing.framework") || "auto";
  
  // For tests with Swift Testing in SPM projects, use swift test command
  // Only use swift test when explicitly set to "swift-testing", not on "auto"
  if (isSPMProject && options.shouldTest && testingFramework === "swift-testing") {
    const packageDir = path.dirname(options.xcworkspace);
    
    context.updateProgressStatus(`Running Swift Testing tests for "${options.scheme}"`);
    
    // Build the swift test command
    const swiftTestArgs = [
      "test",
      "--configuration", options.configuration.toLowerCase(),
    ];
    
    // Add scheme/package target if needed
    if (options.scheme !== "Package") {
      swiftTestArgs.push("--target", options.scheme);
    }
    
    // Add additional args that are compatible with swift test
    const compatibleArgs = additionalArgs.filter(arg => 
      !arg.startsWith("-destination") && 
      !arg.startsWith("-resultBundlePath") &&
      !arg.startsWith("-derivedDataPath")
    );
    swiftTestArgs.push(...compatibleArgs);
    
    let pipes: Command[] | undefined = undefined;
    if (useXcbeatify) {
      pipes = [{ command: "xcbeautify", args: [] }];
    }
    
    // Execute swift test command
    await terminal.execute({
      command: "sh",
      args: ["-c", `cd "${packageDir}" && swift ${swiftTestArgs.map(arg => `"${arg}"`).join(" ")}`],
      pipes: pipes,
      env: env,
    });
    
    await restartSwiftLSP();
    return;
  }
  
  if (isSPMProject) {
    // For SPM projects, we need to run xcodebuild from the package directory
    const packageDir = path.dirname(options.xcworkspace);
    const relativePath = path.relative(getWorkspacePath(), packageDir);
    
    context.updateProgressStatus(`Building SPM package "${options.scheme}"`);
    
    // Build the xcodebuild command for SPM
    const xcodebuildArgs = [
      "-scheme", options.scheme,
      "-configuration", options.configuration,
      "-destination", options.destinationRaw,
    ];
    
    if (options.shouldClean) {
      xcodebuildArgs.push("clean");
    }
    if (options.shouldBuild) {
      xcodebuildArgs.push("build");
    }
    if (options.shouldTest) {
      xcodebuildArgs.push("test");
    }
    
    // Add additional args
    xcodebuildArgs.push(...additionalArgs);
    
    let pipes: Command[] | undefined = undefined;
    if (useXcbeatify) {
      pipes = [{ command: "xcbeautify", args: [] }];
    }
    
    // Execute the command in the package directory
    try {
      await terminal.execute({
        command: "sh",
        args: ["-c", `cd "${packageDir}" && xcodebuild ${xcodebuildArgs.map(arg => `"${arg}"`).join(" ")}`],
        pipes: pipes,
        env: env,
      });
    } catch (error) {
      if (error instanceof Error && isPasscodeProtectionError(error)) {
        handlePasscodeProtectionError(terminal, options.destinationRaw);
        throw new ExtensionError("Device passcode protection prevents build. Please unlock device and trust this computer.");
      }
      throw error;
    }
    
    await restartSwiftLSP();
    return;
  }

  // Original Xcode workspace logic
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

  // Add build settings to reduce device communication for simulator builds
  const skipDeviceConnection = getWorkspaceConfig("build.skipDeviceConnectionForSimulator") ?? false;
  const isSimulatorBuild = options.destinationRaw.includes("Simulator");
  
  if (skipDeviceConnection && isSimulatorBuild) {
    // Disable automatic provisioning updates which can trigger device communication
    command.addBuildSettings("PROVISIONING_PROFILE_SPECIFIER", "");
    command.addBuildSettings("CODE_SIGN_IDENTITY", "");
    command.addBuildSettings("CODE_SIGN_STYLE", "Manual");
    // Skip device-specific entitlements that might trigger device checks
    command.addBuildSettings("SKIP_INSTALL", "NO");
    // Reduce network-based operations during build
    command.addBuildSettings("ENABLE_BITCODE", "NO");
  }

  // For Swift Testing in Xcode projects, add the testing library flag
  if (options.shouldTest && testingFramework === "swift-testing") {
    command.addBuildSettings("SWIFT_TESTING_ENABLED", "YES");
    command.addParameters("-enableTestability", "YES");
  }

  command.addParameters("-scheme", options.scheme);
  command.addParameters("-configuration", options.configuration);
  command.addParameters("-workspace", options.xcworkspace);
  command.addParameters("-destination", options.destinationRaw);
  command.addParameters("-resultBundlePath", bundlePath);
  if (derivedDataPath) {
    command.addParameters("-derivedDataPath", derivedDataPath);
  }
  if (allowProvisioningUpdates) {
    command.addOption("-allowProvisioningUpdates");
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
  if (useXcbeatify) {
    pipes = [{ command: "xcbeautify", args: [] }];
  }

  if (options.shouldClean) {
    context.updateProgressStatus(`Cleaning "${options.scheme}"`);
  } else if (options.shouldBuild) {
    context.updateProgressStatus(`Building "${options.scheme}"`);
  } else if (options.shouldTest) {
    context.updateProgressStatus(`Testing "${options.scheme}" with ${testingFramework === "swift-testing" ? "Swift Testing" : "XCTest"}`);
  }
  
  try {
    await terminal.execute({
      command: commandParts[0],
      args: commandParts.slice(1),
      pipes: pipes,
      env: env,
    });
  } catch (error) {
    if (error instanceof Error && isPasscodeProtectionError(error)) {
      handlePasscodeProtectionError(terminal, options.destinationRaw);
      throw new ExtensionError("Device passcode protection prevents build. Please unlock device and trust this computer.");
    }
    throw error;
  }

  await restartSwiftLSP();

  // Check if periphery scan should run after build
  const runPeripheryAfterBuild = getWorkspaceConfig("periphery.runAfterBuild") ?? false;
  if (runPeripheryAfterBuild && options.shouldBuild) {
    await runPeripheryScan(context, terminal);
  }
}



/**
 * Build app without running
 */
export async function buildCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting build command");
  return commonBuildCommand(context, item, { debug: false });
}

/**
 * Build app in debug mode without running
 */
export async function debuggingBuildCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Building the app (debug mode)");
  return commonBuildCommand(context, item, { debug: true });
}

/**
 * Build app without running
 */
export async function commonBuildCommand(
  context: ExtensionContext,
  item: BuildTreeItem | undefined,
  options: { debug: boolean },
) {
  context.updateProgressStatus("Searching for workspace");
  // If item has a workspace path, use it directly
  const xcworkspace = await askXcodeWorkspacePath(context, item?.workspacePath);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ?? (await askSchemeForBuild(context, { title: "Select scheme to build", xcworkspace: xcworkspace }));

  context.updateProgressStatus("Searching for configuration");
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  context.updateProgressStatus("Searching for destination");
  const destination = await askDestinationToRunOn(context, buildSettings);
  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

  const sdk = destination.platform;

  await runTask(context, {
    name: "Build",
    lock: "sweetpad.build",
    terminateLocked: true,
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      await buildApp(context, terminal, {
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
 * Build and run application on the simulator or device
 */
async function commonLaunchCommand(
  context: ExtensionContext,
  item: BuildTreeItem | undefined,
  options: { debug: boolean },
) {
  context.updateProgressStatus("Searching for workspace");
  // If item has a workspace path, use it directly
  const xcworkspace = await askXcodeWorkspacePath(context, item?.workspacePath);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(context, { title: "Select scheme to build and run", xcworkspace: xcworkspace }));

  context.updateProgressStatus("Searching for configuration");
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  context.updateProgressStatus("Searching for destination");
  const destination = await askDestinationToRunOn(context, buildSettings);

  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

  const sdk = destination.platform;

  const launchArgs = getWorkspaceConfig("build.launchArgs") ?? [];
  const launchEnv = getWorkspaceConfig("build.launchEnv") ?? {};

  await runTask(context, {
    name: options.debug ? "Debug" : "Launch",
    lock: "sweetpad.build",
    terminateLocked: true,
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      await buildApp(context, terminal, {
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
        await runOnMac(context, terminal, {
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
        await runOniOSSimulator(context, terminal, {
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
        await runOniOSDevice(context, terminal, {
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
export async function launchCommand(context: ExtensionContext, item?: BuildTreeItem) {
  // Notify user that build is starting
  vscode.window.showInformationMessage("Launching application... This may take a while.");
  return commonLaunchCommand(context, item, { debug: false });
}

/**
 * Builds and launches the application in debug mode
 * This is a convenience wrapper around launchCommand that sets the debug flag
 */
export async function debuggingLaunchCommand(context: ExtensionContext, item?: BuildTreeItem) {
  return commonLaunchCommand(context, item, { debug: true });
}

/**
 * Run application on the simulator or device without building
 */
export async function runCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting run command");
  vscode.window.showInformationMessage("Running application without building...");
  return commonRunCommand(context, item, { debug: false });
}

/**
 * Run application on the simulator or device without building in debug mode
 */
export async function debuggingRunCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting debugging command");
  return commonRunCommand(context, item, { debug: true });
}

/**
 * Run application on the simulator or device without building
 */
async function commonRunCommand(
  context: ExtensionContext,
  item: BuildTreeItem | undefined,
  options: { debug: boolean },
) {
  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(context, { title: "Select scheme to build and run", xcworkspace: xcworkspace }));

  context.updateProgressStatus("Searching for configuration");
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  context.updateProgressStatus("Searching for destination");
  const destination = await askDestinationToRunOn(context, buildSettings);

  const sdk = destination.platform;

  const launchArgs = getWorkspaceConfig("build.launchArgs") ?? [];
  const launchEnv = getWorkspaceConfig("build.launchEnv") ?? {};

  await runTask(context, {
    name: "Run",
    lock: "sweetpad.build",
    terminateLocked: true,
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      if (destination.type === "macOS") {
        await runOnMac(context, terminal, {
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
        await runOniOSSimulator(context, terminal, {
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
        await runOniOSDevice(context, terminal, {
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
 * Clean build artifacts
 */
export async function cleanCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Searching for workspace");
  // Notify user that cleaning is starting
  vscode.window.showInformationMessage("Cleaning build artifacts... This may take a while.");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ?? (await askSchemeForBuild(context, { title: "Select scheme to clean", xcworkspace: xcworkspace }));

  context.updateProgressStatus("Searching for configuration");
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  context.updateProgressStatus("Searching for destination");
  const destination = await askDestinationToRunOn(context, buildSettings);
  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

  const sdk = destination.platform;

  await runTask(context, {
    name: "Clean",
    lock: "sweetpad.build",
    terminateLocked: true,
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      await buildApp(context, terminal, {
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

/**
 * Run tests for the selected scheme
 */
export async function testCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Searching for workspace");
  vscode.window.showInformationMessage("Starting tests... This may take a while.");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ?? (await askSchemeForBuild(context, { title: "Select scheme to test", xcworkspace: xcworkspace }));

  context.updateProgressStatus("Searching for configuration");
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  context.updateProgressStatus("Searching for destination");
  const destination = await askDestinationToRunOn(context, buildSettings);
  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

  const sdk = destination.platform;

  await runTask(context, {
    name: "Test",
    lock: "sweetpad.build",
    terminateLocked: true,
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      await buildApp(context, terminal, {
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

/**
 * Run tests using Swift Testing framework
 */
export async function testWithSwiftTestingCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Searching for workspace");
  vscode.window.showInformationMessage("Starting Swift Testing tests... This may take a while.");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ?? (await askSchemeForBuild(context, { title: "Select scheme to test with Swift Testing", xcworkspace: xcworkspace }));

  context.updateProgressStatus("Searching for configuration");
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

  // For Swift Testing, we might not need a destination for SPM projects
  const isSPMProject = xcworkspace.endsWith("Package.swift");
  
  if (!isSPMProject) {
    context.updateProgressStatus("Extracting build settings");
    const buildSettings = await getBuildSettingsToAskDestination({
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });

    context.updateProgressStatus("Searching for destination");
    const destination = await askDestinationToRunOn(context, buildSettings);
    const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

    const sdk = destination.platform;

    await runTask(context, {
      name: "Test (Swift Testing)",
      lock: "sweetpad.build",
      terminateLocked: true,
      problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
      callback: async (terminal) => {
        // Temporarily override the testing framework config
        const originalFramework = getWorkspaceConfig("testing.framework");
        await updateWorkspaceConfig("testing.framework", "swift-testing");
        
        try {
          await buildApp(context, terminal, {
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
        } finally {
          // Restore original config
          if (originalFramework !== undefined) {
            await updateWorkspaceConfig("testing.framework", originalFramework);
          }
        }
      },
    });
  } else {
    // For SPM projects, run directly without destination
    await runTask(context, {
      name: "Test (Swift Testing)",
      lock: "sweetpad.build",
      terminateLocked: true,
      problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
      callback: async (terminal) => {
        const packageDir = path.dirname(xcworkspace);
        
        const swiftTestArgs = [
          "test",
          "--configuration", configuration.toLowerCase(),
        ];
        
        if (scheme !== "Package") {
          swiftTestArgs.push("--target", scheme);
        }
        
        const env = getWorkspaceConfig("build.env") || {};
        
        await terminal.execute({
          command: "sh",
          args: ["-c", `cd "${packageDir}" && swift ${swiftTestArgs.map(arg => `"${arg}"`).join(" ")}`],
          env: env,
        });
        
        await restartSwiftLSP();
      },
    });
  }
}

export async function resolveDependencies(
  context: ExtensionContext,
  options: { scheme: string; xcworkspace: string }
) {
  context.updateProgressStatus("Resolving dependencies");
  vscode.window.showInformationMessage(`Resolving dependencies for scheme: ${options.scheme}...`);

  await runTask(context, {
    name: "Resolve Dependencies",
    lock: "sweetpad.build",
    terminateLocked: true,
    callback: async (terminal) => {
      // Handle SPM projects
      if (options.xcworkspace.endsWith("Package.swift")) {
        const packageDir = path.dirname(options.xcworkspace);
        await terminal.execute({
          command: "sh",
          args: ["-c", `cd "${packageDir}" && swift package resolve`],
        });
        return;
      }

      // Original Xcode workspace logic
      await terminal.execute({
        command: "xcodebuild",
        args: ["-resolvePackageDependencies", "-scheme", options.scheme, "-workspace", options.xcworkspace],
      });
    },
  });
}

/**
 * Resolve dependencies for the Xcode project
 */
export async function resolveDependenciesCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Searching for workspace");
  vscode.window.showInformationMessage("Resolving dependencies... This may take a while.");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(context, {
      title: "Select scheme to resolve dependencies",
      xcworkspace: xcworkspace,
    }));

  await resolveDependencies(context, {
    scheme: scheme,
    xcworkspace: xcworkspace,
  });
}

/**
 * Remove directory with build artifacts.
 *
 * Context: we are storing build artifacts in the `build` directory in the storage path for support xcode-build-server.
 */
export async function removeBundleDirCommand(context: ExtensionContext) {
  context.updateProgressStatus("Removing build artifacts directory");
  vscode.window.showInformationMessage("Removing bundle directory...");
  const storagePath = await prepareStoragePath(context);
  const bundleDir = path.join(storagePath, "build");

  await removeDirectory(bundleDir);
  vscode.window.showInformationMessage(`Bundle directory was removed: ${bundleDir}`);
}

/**
 * Generate buildServer.json for xcode-build-server
 * a tool that enable LSP server to see packages from the Xcode project.
 */
export async function generateBuildServerConfigCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting buildServer.json generation");
  vscode.window.showInformationMessage("Generating build server configuration...");

  const isServerInstalled = await getIsXcodeBuildServerInstalled();
  if (!isServerInstalled) {
    throw new ExtensionError("xcode-build-server is not installed");
  }

  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(context, {
      title: "Select scheme for build server",
      xcworkspace: xcworkspace,
    }));

  context.updateProgressStatus("Generating buildServer.json");
  await generateBuildServerConfig({
    xcworkspace: xcworkspace,
    scheme: scheme,
  });
  await restartSwiftLSP();

  const selected = await vscode.window.showInformationMessage("buildServer.json generated in workspace root", "Open");
  if (selected === "Open") {
    const workspacePath = getWorkspacePath();
    const buildServerPath = vscode.Uri.file(path.join(workspacePath, "buildServer.json"));
    await vscode.commands.executeCommand("vscode.open", buildServerPath);
  }
  context.simpleTaskCompletionEmitter.fire();
}

/**
 * Open current project in Xcode
 */
export async function openXcodeCommand(context: ExtensionContext) {
  context.updateProgressStatus("Opening project in Xcode");
  vscode.window.showInformationMessage("Opening project in Xcode...");
  const xcworkspace = await askXcodeWorkspacePath(context);

  await exec({
    command: "open",
    args: [xcworkspace],
  });
}

/**
 * Select Xcode workspace and save it to the workspace state
 */
export async function selectXcodeWorkspaceCommand(context: ExtensionContext, item?: WorkspaceGroupTreeItem) {
  context.updateProgressStatus("Searching for workspace");
  
  if (item) {
    // Set loading state on this specific item only
    item.setLoading(true);
    
    try {
      let path = item.workspacePath;
      if (path) {
        // Update the workspace path without triggering a full refresh
        context.buildManager.setCurrentWorkspacePath(path, true); // Skip refresh
        context.updateWorkspaceState("build.xcodeWorkspacePath", path);
      }
      
      // Short delay to allow UI to update with loading state
      await new Promise(resolve => setTimeout(resolve, 300));
      
      // Show success message
      vscode.window.showInformationMessage(`Workspace path updated`);
    } finally {
      // Allow a moment for the success message to be seen
      await new Promise(resolve => setTimeout(resolve, 500));
      
      // Clear loading state
      item.setLoading(false);
      
      // Add a small delay to ensure UI has time to update
      await new Promise(resolve => setTimeout(resolve, 100));
      
      // Now refresh the build manager
      context.buildManager.refresh();
    }
    return;
  }

  // Manual selection via quick pick
  vscode.window.showInformationMessage("Selecting Xcode workspace...");
  const workspace = await selectXcodeWorkspace({
    autoselect: false,
  });

  if (workspace) {
    context.updateWorkspaceState("build.xcodeWorkspacePath", workspace);
  }
  
  context.buildManager.refresh();
  context.simpleTaskCompletionEmitter.fire();
}

export async function selectXcodeSchemeForBuildCommand(context: ExtensionContext, item?: BuildTreeItem) {
  vscode.window.showInformationMessage("Selecting Xcode scheme for build...");
  
  if (item) {
    item.provider.buildManager.setDefaultSchemeForBuild(item.scheme);
    return;
  }

  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  await askSchemeForBuild(context, {
    title: "Select scheme to set as default",
    xcworkspace: xcworkspace,
    ignoreCache: true,
  });

  context.simpleTaskCompletionEmitter.fire();
}

/**
 * Ask user to select configuration for build and save it to the build manager cache
 */
export async function selectConfigurationForBuildCommand(context: ExtensionContext): Promise<void> {
  context.updateProgressStatus("Searching for workspace");
  vscode.window.showInformationMessage("Selecting build configuration...");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for configurations");
  const configurations = await getBuildConfigurations({
    xcworkspace: xcworkspace,
  });

  let selected: string | undefined;
  if (configurations.length === 0) {
    selected = await vscode.window.showInputBox({
      title: "No configurations found. Please enter configuration name manually",
    });
  } else {
    selected = await showConfigurationPicker(configurations);
  }

  if (!selected) {
    vscode.window.showErrorMessage("Configuration was not selected");
    return;
  }

  const saveAnswer = await showYesNoQuestion({
    title: "Do you want to update configuration in the workspace settings (.vscode/settings.json)?",
  });
  if (saveAnswer) {
    await updateWorkspaceConfig("build.configuration", selected);
    context.buildManager.setDefaultConfigurationForBuild(undefined);
  } else {
    context.buildManager.setDefaultConfigurationForBuild(selected);
  }
}

export async function diagnoseBuildSetupCommand(context: ExtensionContext): Promise<void> {
  context.updateProgressStatus("Diagnosing build setup");
  vscode.window.showInformationMessage("Diagnosing build setup...");

  await runTask(context, {
    name: "Diagnose Build Setup",
    lock: "sweetpad.build",
    terminateLocked: true,
    callback: async (terminal) => {
      const _write = (message: string) =>
        terminal.write(message, {
          newLine: true,
        });

      const _writeQuote = (message: string) => {
        const splited = message.split("\n");
        for (const line of splited) {
          _write(`   ${line}`);
        }
      };

      _write("SweetPad: Diagnose Build Setup");
      _write("================================");

      const hostPlatform = process.platform;
      _write("🔎 Checking OS");
      if (hostPlatform !== "darwin") {
        _write(
          `❌ Host platform ${hostPlatform} is not supported. This extension depends on Xcode which is available only on macOS`,
        );
        return;
      }
      _write(`✅ Host platform: ${hostPlatform}\n`);
      _write("================================");

      const workspacePath = getWorkspacePath();
      _write("🔎 Checking VS Code workspace path");
      _write(`✅ VSCode workspace path: ${workspacePath}\n`);
      _write("================================");

      const xcWorkspacePath = getCurrentXcodeWorkspacePath(context);
      _write("🔎 Checking current xcode worskpace path");
      _write(`✅ Xcode workspace path: ${xcWorkspacePath ?? "<project-root>"}\n`);
      _write("================================");

      const currentScheme = context.getWorkspaceState("build.xcodeScheme");
      _write("🔎 Checking current xcode scheme");
      _write(`✅ Xcode scheme: ${currentScheme ?? "<default>"}\n`);
      _write("================================");

      _write("🔎 Getting schemes");
      let schemes: XcodeScheme[] = [];
      try {
        schemes = await context.buildManager.getSchemas({ refresh: true });
      } catch (e) {
        _write("❌ Getting schemes failed");
        if (e instanceof ExecBaseError) {
          const strerr = e.options?.context?.stderr as string | undefined;
          if (
            strerr?.startsWith("xcode-select: error: tool 'xcodebuild' requires Xcode, but active developer directory")
          ) {
            _write("❌ Xcode build tools are not activated");
            const isXcodeExists = await isFileExists("/Applications/Xcode.app");
            if (!isXcodeExists) {
              _write("❌ Xcode is not installed");
              _write("🌼 Try this:");
              _write("   1. Download Xcode from App Store https://appstore.com/mac/apple/xcode");
              _write("   2. Accept the Terms and Conditions");
              _write("   3. Ensure Xcode app is in the /Applications directory (NOT /Users/{user}/Applications)");
              _write("   4. Run command `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`");
              _write("   5. Restart VS Code");
              _write("🌼 See more: https://stackoverflow.com/a/17980786/7133756");
              return;
            }
            _write("✅ Xcode is installed and located in /Applications/Xcode.app");
            _write("🌼 Try to activate Xcode:");
            _write("   1. Execute this command `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`");
            _write("   2. Restart VS Code");
            _write("🌼 See more: https://stackoverflow.com/a/17980786/7133756\n");
            return;
          }
          if (strerr?.includes("does not contain an Xcode project, workspace or package")) {
            _write("❌ Xcode workspace not found");
            _write("❌ Error message from xcodebuild:");
            context.simpleTaskCompletionEmitter.fire();
            _writeQuote(strerr);
            _write(
              "🌼 Check whether your project folder contains folders with the extensions .xcodeproj or .xcworkspace",
            );
            const xcodepaths = await detectXcodeWorkspacesPaths();
            if (xcodepaths.length > 0) {
              _write("✅ Found Xcode and Xcode workspace paths:");
              for (const path of xcodepaths) {
                _write(`   - ${path}`);
              }
            }
            return;
          }
          _write("❌ Error message from xcodebuild:");
          context.simpleTaskCompletionEmitter.fire();
          _writeQuote(strerr ?? "Unknown error");
          return;
        }
        _write("❌ Error message from xcodebuild:");
        context.simpleTaskCompletionEmitter.fire();
        _writeQuote(e instanceof Error ? e.message : String(e));
        return;
      }
      if (schemes.length === 0) {
        _write("❌ No schemes found");
        context.simpleTaskCompletionEmitter.fire();
        return;
      }

      _write(`✅ Found ${schemes.length} schemes\n`);
      _write("   Schemes:");
      for (const scheme of schemes) {
        _write(`   - ${scheme.name}`);
      }
      _write("================================");

      _write("✅ Everything looks good!");
      context.simpleTaskCompletionEmitter.fire();
    },
  });
}

/**
 * Run periphery scan to detect unused code
 */
export async function runPeripheryScan(
  context: ExtensionContext,
  terminal: TaskTerminal,
) {
  context.updateProgressStatus("Running Periphery scan");
  terminal.write("🔍 Starting Periphery scan for unused code...\n");
  terminal.write("📋 Default rules enabled: retain public, objc-accessible\n");

  // Check if periphery is installed
  try {
    await exec({
      command: "periphery",
      args: ["version"],
    });
  } catch (error) {
    terminal.write("❌ Periphery is not installed. Install it using: brew install periphery\n");
    throw new ExtensionError("Periphery is not installed");
  }

  // Get derived data path and construct index store path
  const derivedDataPath = prepareDerivedDataPath();
  let indexStorePath: string;
  
  if (derivedDataPath) {
    indexStorePath = path.join(derivedDataPath, "Index.noindex", "DataStore");
  } else {
    // Use default Xcode derived data location - look for project-specific folder
    const defaultDerivedDataPath = path.join(process.env.HOME || "~", "Library", "Developer", "Xcode", "DerivedData");
    
    // Try to find project-specific derived data folder
    try {
      const derivedDataFolders = await readdir(defaultDerivedDataPath);
      const projectFolders = derivedDataFolders.filter((folder: string) => 
        folder.includes("DoordashAttestation") || folder.includes("Package")
      );
      
      if (projectFolders.length > 0) {
        // Use the first matching project folder
        indexStorePath = path.join(defaultDerivedDataPath, projectFolders[0], "Index.noindex", "DataStore");
      } else {
        // Fallback to generic path
        indexStorePath = path.join(defaultDerivedDataPath, "Index.noindex", "DataStore");
      }
    } catch (error) {
      // If we can't read the derived data folder, use generic path
      indexStorePath = path.join(defaultDerivedDataPath, "Index.noindex", "DataStore");
    }
  }

  // Check if index store path exists
  const indexStoreExists = await isFileExists(indexStorePath);
  if (!indexStoreExists) {
    terminal.write(`❌ Index store path does not exist: ${indexStorePath}\n`);
    terminal.write("💡 Make sure you have built the project first to generate the index store.\n");
    terminal.write("💡 You can run 'Build' first, then 'Periphery Scan', or use 'Build & Periphery Scan'.\n");
    
    // Show available derived data folders if possible
    try {
      const defaultDerivedDataPath = path.join(process.env.HOME || "~", "Library", "Developer", "Xcode", "DerivedData");
      const derivedDataFolders = await readdir(defaultDerivedDataPath);
      if (derivedDataFolders.length > 0) {
        terminal.write("📁 Available derived data folders:\n");
        derivedDataFolders.slice(0, 5).forEach((folder: string) => {
          terminal.write(`   - ${folder}\n`);
        });
      }
    } catch (error) {
      // Ignore error in showing available folders
    }
    
    throw new ExtensionError("Index store path does not exist. Build the project first.");
  }

  // Build periphery scan command
  const peripheryArgs = [
    "scan",
    "--skip-build",
    "--index-store-path", indexStorePath,
  ];

  // Add default rules to retain public declarations (can be overridden by config)
  const retainPublic = getWorkspaceConfig("periphery.retainPublic") ?? true;
  if (retainPublic) {
    peripheryArgs.push("--retain-public");
  }

  // Add rule to retain objc accessible declarations
  const retainObjcAccessible = getWorkspaceConfig("periphery.retainObjcAccessible") ?? true;
  if (retainObjcAccessible) {
    peripheryArgs.push("--retain-objc-accessible");
  }



  // Check for .periphery.yml file in project root first
  const projectRoot = getWorkspacePath();
  const defaultPeripheryConfigPath = path.join(projectRoot, ".periphery.yml");
  
  let peripheryConfigPath: string | undefined;
  
  // First check if .periphery.yml exists in project root
  const defaultConfigExists = await isFileExists(defaultPeripheryConfigPath);
  if (defaultConfigExists) {
    peripheryConfigPath = defaultPeripheryConfigPath;
    terminal.write(`📋 Using .periphery.yml from project root\n`);
  } else {
    // Check workspace config for custom path
    const workspaceConfig = getWorkspaceConfig("periphery.config");
    if (workspaceConfig) {
      peripheryConfigPath = workspaceConfig;
      terminal.write(`📋 Using custom periphery config: ${peripheryConfigPath}\n`);
    } else {
      // Ask user for config path
      const userConfigPath = await vscode.window.showInputBox({
        title: "Periphery Configuration",
        prompt: "Enter path to .periphery.yml file (or leave empty to use default settings)",
        placeHolder: ".periphery.yml",
        value: "",
      });
      
      if (userConfigPath && userConfigPath.trim()) {
        const resolvedPath = path.isAbsolute(userConfigPath) 
          ? userConfigPath 
          : path.join(projectRoot, userConfigPath);
        
        const configExists = await isFileExists(resolvedPath);
        if (configExists) {
          peripheryConfigPath = resolvedPath;
          terminal.write(`📋 Using periphery config: ${peripheryConfigPath}\n`);
        } else {
          terminal.write(`⚠️  Config file not found: ${resolvedPath}\n`);
          terminal.write(`📋 Using default settings\n`);
        }
      } else {
        terminal.write(`📋 Using default settings\n`);
      }
    }
  }
  
  // Add config parameter if we have a valid path
  if (peripheryConfigPath) {
    peripheryArgs.push("--config", peripheryConfigPath);
  }

  // Add format option if specified
  const outputFormat = getWorkspaceConfig("periphery.format") || "xcode";
  peripheryArgs.push("--format", outputFormat);

  // Add quiet option if specified
  const quiet = getWorkspaceConfig("periphery.quiet") ?? false;
  if (quiet) {
    peripheryArgs.push("--quiet");
  }

  try {
    await terminal.execute({
      command: "periphery",
      args: peripheryArgs,
    });
    terminal.write("✅ Periphery scan completed successfully!\n");
  } catch (error) {
    terminal.write("⚠️  Periphery scan completed with findings\n");
    // Don't throw error as periphery exits with non-zero code when it finds unused code
  }
}

/**
 * Build and run periphery scan
 */
export async function buildAndPeripheryScanCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting build and periphery scan");
  
  // Get build configuration
  const xcworkspace = await askXcodeWorkspacePath(context, item?.workspacePath);
  const scheme = item?.scheme ?? (await askSchemeForBuild(context, { title: "Select scheme for periphery scan", xcworkspace: xcworkspace }));
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });
  
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  const destination = await askDestinationToRunOn(context, buildSettings);
  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });
  const sdk = destination.platform;

  await runTask(context, {
    name: "Build & Periphery Scan",
    lock: "sweetpad.build",
    terminateLocked: true,
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      // First build the project
      await buildApp(context, terminal, {
        scheme: scheme,
        sdk: sdk,
        configuration: configuration,
        shouldBuild: true,
        shouldClean: false,
        shouldTest: false,
        xcworkspace: xcworkspace,
        destinationRaw: destinationRaw,
        debug: false,
      });

      // Then run periphery scan
      await runPeripheryScan(context, terminal);
    },
  });
}

/**
 * Run periphery scan only (without building)
 */
export async function peripheryScanCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting periphery scan");
  
  // Get build configuration  
  const xcworkspace = await askXcodeWorkspacePath(context, item?.workspacePath);
  const scheme = item?.scheme ?? (await askSchemeForBuild(context, { title: "Select scheme for periphery scan", xcworkspace: xcworkspace }));
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });
  
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  const destination = await askDestinationToRunOn(context, buildSettings);
  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });
  const sdk = destination.platform;

  await runTask(context, {
    name: "Periphery Scan",
    lock: "sweetpad.periphery",
    terminateLocked: true,
    callback: async (terminal) => {
      await runPeripheryScan(context, terminal);
    },
  });
}

/**
 * Create a .periphery.yml configuration file template
 */
export async function createPeripheryConfigCommand(context: ExtensionContext) {
  const projectRoot = getWorkspacePath();
  const peripheryConfigPath = path.join(projectRoot, ".periphery.yml");
  
  // Check if .periphery.yml already exists
  const configExists = await isFileExists(peripheryConfigPath);
  if (configExists) {
    const overwrite = await vscode.window.showWarningMessage(
      ".periphery.yml already exists. Do you want to overwrite it?",
      "Overwrite",
      "Cancel"
    );
    
    if (overwrite !== "Overwrite") {
      return;
    }
  }
  
  // Get current workspace info for template
  const xcworkspace = getCurrentXcodeWorkspacePath(context);
  const workspaceName = xcworkspace ? path.basename(xcworkspace, path.extname(xcworkspace)) : "YourProject";
  
  // Create template content
  const templateContent = `# Periphery Configuration File
# See https://github.com/peripheryapp/periphery for more options

# Project configuration
workspace: ${xcworkspace || "YourProject.xcworkspace"}
# project: YourProject.xcodeproj  # Use this for single project setup

# Build configuration
clean_build: true
schemes:
  - ${workspaceName}
targets:
  - ${workspaceName}

# Retention options
retain_public: true
retain_objc_accessible: true
retain_unused_protocol_func_params: true

# Output configuration
format: xcode
quiet: false
verbose: false

# File exclusions (optional)
# report_exclude:
#   - "*/Generated/*"
#   - "*/Pods/*"
#   - "*/Tests/*"

# Index exclusions (optional)
# index_exclude:
#   - "*/Generated/*"
#   - "*/Pods/*"
`;

  try {
    await vscode.workspace.fs.writeFile(
      vscode.Uri.file(peripheryConfigPath),
      Buffer.from(templateContent, 'utf8')
    );
    
    vscode.window.showInformationMessage(
      `✅ Created .periphery.yml configuration file at project root`
    );
    
    // Open the created file
    const document = await vscode.workspace.openTextDocument(peripheryConfigPath);
    await vscode.window.showTextDocument(document);
    
  } catch (error) {
    vscode.window.showErrorMessage(`❌ Failed to create .periphery.yml: ${error}`);
  }
}