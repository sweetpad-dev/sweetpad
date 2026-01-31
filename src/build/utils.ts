import path from "node:path";
import * as vscode from "vscode";
import { type QuickPickItem, showQuickPick } from "../common/quick-pick";

import { askConfigurationBase } from "../common/askers";
import {
  type XcodeBuildServerConfig,
  generateBuildServerConfig,
  getBuildSettingsToAskDestination,
  getIsXcodeBuildServerInstalled,
  getSchemes,
  readXcodeBuildServerConfig,
} from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { ExtensionError } from "../common/errors";
import { createDirectory, findFilesRecursive, isFileExists, removeDirectory } from "../common/files";
import { commonLogger } from "../common/logger";
import type { TaskTerminal } from "../common/tasks";
import { assertUnreachable } from "../common/types";
import type { DestinationPlatform } from "../destination/constants";
import type { Destination } from "../destination/types";
import { splitSupportedDestinatinos } from "../destination/utils";
import type { SimulatorDestination } from "../simulators/types";

export type SelectedDestination = {
  type: "simulator" | "device";
  udid: string;
  name?: string;
};

/**
 * Ask user to select one of the Booted/Shutdown simulators
 */
export async function askSimulator(
  context: ExtensionContext,
  options: {
    title: string;
    state: "Booted" | "Shutdown";
    error: string;
  },
): Promise<SimulatorDestination> {
  let simulators = await context.destinationsManager.getSimulators({
    sort: true,
  });

  if (options?.state) {
    simulators = simulators.filter((simulator) => simulator.state === options.state);
  }

  if (simulators.length === 0) {
    throw new ExtensionError(options.error);
  }
  if (simulators.length === 1) {
    return simulators[0];
  }

  const selected = await showQuickPick({
    title: options.title,
    items: simulators.map((simulator) => {
      return {
        label: simulator.label,
        context: {
          simulator: simulator,
        },
      };
    }),
  });

  return selected.context.simulator;
}

/**
 * Ask user to select simulator or device to run on
 */
export async function askDestinationToRunOn(
  context: ExtensionContext,
  options: {
    scheme: string;
    configuration: string;
    sdk: string | undefined;
    xcworkspace: string;
  },
): Promise<Destination> {
  context.updateProgressStatus("Searching for destinations");
  const destinations = await context.destinationsManager.getDestinations({
    mostUsedSort: true,
  });

  // If we have cached desination, use it
  const cachedDestination = context.destinationsManager.getSelectedXcodeDestinationForBuild();
  if (cachedDestination) {
    const destination = destinations.find(
      (destination) => destination.id === cachedDestination.id && destination.type === cachedDestination.type,
    );
    if (destination) {
      return destination;
    }
  }

  // We can remove platforms that are not supported by the build settings
  // WARNING: if want to avoid refetching build settings, move this logic to build manager or build context (not exist yet)
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: options.sdk,
    xcworkspace: options.xcworkspace,
  });
  const supportedPlatforms = buildSettings?.supportedPlatforms;

  return await selectDestinationForBuild(context, {
    destinations: destinations,
    supportedPlatforms: supportedPlatforms,
  });
}

export async function selectDestinationForBuild(
  context: ExtensionContext,
  options: {
    destinations: Destination[];
    supportedPlatforms: DestinationPlatform[] | undefined;
  },
): Promise<Destination> {
  const { supported, unsupported } = splitSupportedDestinatinos({
    destinations: options.destinations,
    supportedPlatforms: options.supportedPlatforms,
  });

  const supportedItems: QuickPickItem<Destination>[] = supported.map((destination) => ({
    label: destination.name,
    iconPath: new vscode.ThemeIcon(destination.icon),
    detail: destination.quickPickDetails,
    context: destination,
  }));
  const unsupportedItems: QuickPickItem<Destination>[] = unsupported.map((destination) => ({
    label: destination.name,
    iconPath: new vscode.ThemeIcon(destination.icon),
    detail: destination.quickPickDetails,
    context: destination,
  }));

  const items: QuickPickItem<Destination>[] = [];
  if (unsupported.length === 0 && supported.length === 0) {
    // Show that no destinations found
    items.push({
      label: "No destinations found",
      kind: vscode.QuickPickItemKind.Separator,
    });
  } else if (supported.length > 0 && unsupported.length > 0) {
    // Split supported and unsupported destinations
    items.push({
      label: "Supported platforms",
      kind: vscode.QuickPickItemKind.Separator,
    });
    items.push(...supportedItems);
    items.push({
      label: "Other",
      kind: vscode.QuickPickItemKind.Separator,
    });
    items.push(...unsupportedItems);
  } else {
    // Just make flat list, one is empty and another is not
    items.push(...supportedItems);
    items.push(...unsupportedItems);
  }

  const selected = await showQuickPick<Destination>({
    title: "Select destination to run on",
    items: items,
  });

  const destination = selected.context;

  context.destinationsManager.setWorkspaceDestinationForBuild(destination);
  return destination;
}

/**
 * Ask user to select scheme to build
 */
export async function askSchemeForBuild(
  context: ExtensionContext,
  options: {
    title?: string;
    xcworkspace: string;
    ignoreCache?: boolean;
  },
): Promise<string> {
  context.updateProgressStatus("Searching for scheme");

  const cachedScheme = context.buildManager.getDefaultSchemeForBuild();
  if (cachedScheme && !options.ignoreCache) {
    return cachedScheme;
  }

  const schemes = await getSchemes({
    xcworkspace: options.xcworkspace,
  });

  const scheme = await showQuickPick({
    title: options?.title ?? "Select scheme to build",
    items: schemes.map((scheme) => {
      return {
        label: scheme.name,
        context: {
          scheme: scheme,
        },
      };
    }),
  });

  const schemeName = scheme.context.scheme.name;
  context.buildManager.setDefaultSchemeForBuild(schemeName);
  return schemeName;
}

/**
 * It's absolute path to current opened workspace
 */
export function getWorkspacePath(): string {
  const workspaceFolder = vscode.workspace.workspaceFolders?.[0].uri.fsPath;
  if (!workspaceFolder) {
    throw new ExtensionError("No workspace folder found");
  }
  return workspaceFolder;
}

/**
 * Prepare storage path for the extension. It's a folder where we store all intermediate files
 */
export async function prepareStoragePath(context: ExtensionContext): Promise<string> {
  const storagePath = context.storageUri?.fsPath;
  if (!storagePath) {
    throw new ExtensionError("No storage path found");
  }
  // Creatre folder at storagePath, because vscode doesn't create it automatically
  await createDirectory(storagePath);
  return storagePath;
}

/**
 * Prepare bundle directory for the given scheme in the storage path
 */
export async function prepareBundleDir(context: ExtensionContext, scheme: string): Promise<string> {
  const storagePath = await prepareStoragePath(context);

  const bundleDir = path.join(storagePath, "bundle", scheme);

  // Remove old bundle if exists
  await removeDirectory(bundleDir);

  // Remove old .xcresult if exists
  const xcresult = path.join(storagePath, "bundle", `${scheme}.xcresult`);
  await removeDirectory(xcresult);

  return bundleDir;
}

export function prepareDerivedDataPath(): string | null {
  const configPath = getWorkspaceConfig("build.derivedDataPath");

  // No config -> path will be provided by xcodebuild
  if (!configPath) {
    return null;
  }

  // Expand relative path to absolute
  let derivedDataPath: string = configPath;
  if (!path.isAbsolute(configPath)) {
    // Example: .biuld/ -> /Users/username/Projects/project/.build
    derivedDataPath = path.join(getWorkspacePath(), configPath);
  }

  return derivedDataPath;
}

export function getCurrentXcodeWorkspacePath(context: ExtensionContext): string | undefined {
  const configPath = getWorkspaceConfig("build.xcodeWorkspacePath");
  if (configPath) {
    context.updateWorkspaceState("build.xcodeWorkspacePath", undefined);
    if (path.isAbsolute(configPath)) {
      return configPath;
    }
    return path.join(getWorkspacePath(), configPath);
  }

  const cachedPath = context.getWorkspaceState("build.xcodeWorkspacePath");
  if (cachedPath) {
    return cachedPath;
  }

  return undefined;
}

export async function askXcodeWorkspacePath(context: ExtensionContext): Promise<string> {
  const current = getCurrentXcodeWorkspacePath(context);
  if (current) {
    return current;
  }

  const selectedPath = await selectXcodeWorkspace({
    autoselect: true,
  });

  context.updateWorkspaceState("build.xcodeWorkspacePath", selectedPath);
  context.buildManager.refreshSchemes();
  return selectedPath;
}

export async function askConfiguration(
  context: ExtensionContext,
  options: {
    xcworkspace: string;
  },
): Promise<string> {
  context.updateProgressStatus("Searching for build configuration");

  const fromConfig = getWorkspaceConfig("build.configuration");
  if (fromConfig) {
    return fromConfig;
  }
  const cached = context.buildManager.getDefaultConfigurationForBuild();
  if (cached) {
    return cached;
  }
  const selected = await askConfigurationBase({
    xcworkspace: options.xcworkspace,
  });
  context.buildManager.setDefaultConfigurationForBuild(selected);
  return selected;
}

/**
 * Check if buildServer.json needs to be regenerated and regenerate it if needed
 */
export async function generateBuildServerConfigOnBuild(options: {
  scheme: string;
  xcworkspace: string;
}): Promise<void> {
  const isEnabled = getWorkspaceConfig("xcodebuildserver.autogenerate") ?? true;
  if (!isEnabled) {
    return;
  }

  const isServerInstalled = await getIsXcodeBuildServerInstalled();
  if (!isServerInstalled) {
    return;
  }

  let config: XcodeBuildServerConfig | undefined = undefined;
  try {
    config = await readXcodeBuildServerConfig();
  } catch (e) {
    // regenerate config in case of errors like JSON invalid or file does not exist
  }

  // regenegerate config only if something is wrong with config:
  // - scheme does not match
  // - workspace does not exist
  // - build_root does not exist
  const isConfigValid =
    config &&
    config.scheme === options.scheme &&
    config.workspace &&
    config.build_root &&
    (await isFileExists(config.build_root)) &&
    (await isFileExists(config.workspace));

  if (!isConfigValid) {
    await generateBuildServerConfig({
      xcworkspace: options.xcworkspace,
      scheme: options.scheme,
    });
    await restartSwiftLSP();
  }
}

/**
 * Detect xcode workspace in the given directory
 */
export async function detectXcodeWorkspacesPaths(): Promise<string[]> {
  const workspace = getWorkspacePath();

  // Get all files that end with .xcworkspace (4 depth)
  const paths = await findFilesRecursive({
    directory: workspace,
    depth: 4,
    matcher: (file) => {
      return file.name.endsWith(".xcworkspace");
    },
  });
  return paths;
}

/**
 * Find xcode workspace in the given directory and ask user to select it
 */
export async function selectXcodeWorkspace(options: { autoselect: boolean }): Promise<string> {
  const workspacePath = getWorkspacePath();

  // Get all files that end with .xcworkspace (4 depth)
  const paths = await detectXcodeWorkspacesPaths();

  // No files, nothing to do
  if (paths.length === 0) {
    throw new ExtensionError("No xcode workspaces found", {
      context: {
        cwd: workspacePath,
      },
    });
  }

  // One file, use it and save it to the cache
  if (paths.length === 1 && options.autoselect) {
    const path = paths[0];
    commonLogger.log("Xcode workspace was detected", {
      workspace: workspacePath,
      path: path,
    });
    return path;
  }

  const podfilePath = path.join(workspacePath, "Podfile");
  const isCocoaProject = await isFileExists(podfilePath);

  // More then one, ask user to select
  const selected = await showQuickPick({
    title: "Select xcode workspace",
    items: paths
      .sort((a, b) => {
        // Sort by depth to show less nested paths first
        const aDepth = a.split(path.sep).length;
        const bDepth = b.split(path.sep).length;
        return aDepth - bDepth;
      })
      .map((xwPath) => {
        // show only relative path, to make it more readable
        const relativePath = path.relative(workspacePath, xwPath);
        const parentDir = path.dirname(relativePath);

        const isInRootDir = parentDir === ".";
        const isCocoaPods = isInRootDir && isCocoaProject;

        let detail: string | undefined;
        if (isCocoaPods && isInRootDir) {
          detail = "CocoaPods (recommended)";
        } else if (!isInRootDir && parentDir.endsWith(".xcodeproj")) {
          detail = "Xcode";
        }
        // todo: add workspace with multiple projects

        return {
          label: relativePath,
          detail: detail,
          context: {
            path: xwPath,
          },
        };
      }),
  });
  return selected.context.path;
}

export async function restartSwiftLSP() {
  // Restart SourceKit Language Server
  try {
    await vscode.commands.executeCommand("swift.restartLSPServer");
  } catch (error) {
    commonLogger.warn("Error restarting SourceKit Language Server", {
      error: error,
    });
  }
}

export function isXcbeautifyEnabled() {
  return getWorkspaceConfig("build.xcbeautifyEnabled") ?? true;
}

export class XcodeCommandBuilder {
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
    // Cases:
    // ["-arg1", "value1", "-arg2", "value2", "-arg3", "-arg4", "value4"]
    // ["xcodebuild", "-arg1", "value1", "-arg2", "value2", "-arg3", "-arg4", "value4"]
    // ["ARG1=value1", "ARG2=value2", "ARG3", "ARG4=value4"]
    // ["xcodebuild", "ARG1=value1", "ARG2=value2", "ARG3", "ARG4=value4"]
    if (args.length === 0) {
      return;
    }

    for (let i = 0; i < args.length; i++) {
      const current = args[i];
      const next = args[i + 1];
      if (current && next && current.startsWith("-") && !next.startsWith("-")) {
        this.parameters.push({
          arg: current,
          value: next,
        });
        i++;
      } else if (current?.startsWith("-")) {
        this.parameters.push({
          arg: current,
          value: this.NO_VALUE,
        });
      } else if (current?.includes("=")) {
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

/**
 * Prepare and return destination string for xcodebuild command.
 *
 * WARN: Do not use result of this function to anything else than xcodebuild command.
 */
export function getXcodeBuildDestinationString(options: { destination: Destination }): string {
  const destination = options.destination;

  if (destination.type === "iOSSimulator") {
    const arch = getSimulatorArch();
    return buildDestinationString({ platform: "iOS Simulator", id: destination.udid, arch: arch });
  }
  if (destination.type === "watchOSSimulator") {
    const arch = getSimulatorArch();
    return buildDestinationString({ platform: "watchOS Simulator", id: destination.udid, arch: arch });
  }
  if (destination.type === "tvOSSimulator") {
    const arch = getSimulatorArch();
    return buildDestinationString({ platform: "tvOS Simulator", id: destination.udid, arch: arch });
  }
  if (destination.type === "visionOSSimulator") {
    const arch = getSimulatorArch();
    return buildDestinationString({ platform: "visionOS Simulator", id: destination.udid, arch: arch });
  }
  if (destination.type === "macOS") {
    // note: without arch, xcodebuild will show warning like this:
    // --- xcodebuild: WARNING: Using the first of multiple matching destinations:
    // { platform:macOS, arch:arm64, id:00008103-000109910EC3001E, name:My Mac }
    // { platform:macOS, arch:x86_64, id:00008103-000109910EC3001E, name:My Mac }
    // return `platform=macOS,arch=${destination.arch}`;
    return buildDestinationString({ platform: "macOS", arch: destination.arch });
  }
  if (destination.type === "iOSDevice") {
    return buildDestinationString({ platform: "iOS", id: destination.udid });
  }
  if (destination.type === "watchOSDevice") {
    return buildDestinationString({ platform: "watchOS", id: destination.udid });
  }
  if (destination.type === "tvOSDevice") {
    return buildDestinationString({ platform: "tvOS", id: destination.udid });
  }
  if (destination.type === "visionOSDevice") {
    return buildDestinationString({ platform: "visionOS", id: destination.udid });
  }
  return assertUnreachable(destination);
}

/**
 * Build destination string for xcodebuild command.
 *
 * Examples:
 * - `platform=iOS Simulator,id=12345678-1234-1234-1234-123456789012,arch=x86_64`
 * - `platform=macOS,arch=arm64`
 * - `platform=iOS,arch=arm64`
 */
function buildDestinationString(options: {
  platform: string;
  id?: string;
  arch?: string;
}): string {
  const { platform, id, arch } = options;
  if (id && arch) {
    return `platform=${platform},id=${id},arch=${arch}`;
  }
  if (id && !arch) {
    return `platform=${platform},id=${id}`;
  }
  if (!id && arch) {
    return `platform=${platform},arch=${arch}`;
  }
  return `platform=${platform}`; // no id and no arch
}

function getSimulatorArch(): string | undefined {
  // Rosetta is technology that allows running x86_64 code on Apple Silicon Macs.
  // This function instructs xcodebuild to build for x86_64 architecture when Rosetta destinations
  // enabled in Xcode
  const useRosetta = getWorkspaceConfig("build.rosettaDestination") ?? false;
  if (useRosetta) {
    return "x86_64";
  }
  return undefined; // let xcodebuild decide the architecture
}

export function writeWatchMarkers(terminal: TaskTerminal) {
  terminal.write("🍭 SweetPad: watch marker (start)\n");
  terminal.write("🍩 SweetPad: watch marker (end)\n\n");
}

export async function ensureAppPathExists(appPath: string | undefined): Promise<string> {
  if (!appPath) {
    throw new ExtensionError("App path is empty. Something went wrong.");
  }

  const isExists = await isFileExists(appPath);
  if (!isExists) {
    throw new ExtensionError(`App path does not exist. Have you built the app? Path: ${appPath}`);
  }
  return appPath;
}
