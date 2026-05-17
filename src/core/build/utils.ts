import path from "node:path";

import { execa } from "execa";

import type { UserAsker } from "../asker/types";
import { askConfigurationBase } from "../askers";
import {
  type XcodeBuildServerConfig,
  type XcodeCliDeps,
  detectWorkspaceType,
  generateBuildServerConfig,
  getBuildSettingsToAskDestination,
  getIsXcodeBuildServerInstalled,
  getSchemes,
  getXcodeBuildCommand,
  readXcodeBuildServerConfig,
} from "../cli/scripts";
import type { ConfigProvider } from "../config/types";
import type { DestinationPlatform } from "../destination/constants";
import type { DestinationsManager } from "../destination/manager";
import type { Destination } from "../destination/types";
import { splitSupportedDestinatinos } from "../destination/utils";
import { ExtensionError } from "../errors";
import { createDirectory, findFilesRecursive, isFileExists, removeDirectory } from "../files";
import type { Logger } from "../logger/types";
import type { LspRefresher } from "../lsp/types";
import type { ProgressReporter } from "../progress";
import type { SimulatorDestination } from "../simulators/types";
import type { WorkspaceState } from "../state/types";
import type { TaskTerminal } from "../tasks/types";
import { assertUnreachable } from "../types";
import { XcodeWorkspace } from "../xcode/workspace";
import { LaunchAction } from "../xcode/xcscheme";
import type { BuildManager } from "./manager";

export { detectWorkspaceType, getSwiftPMDirectory } from "../cli/scripts";

export type SelectedDestination = {
  type: "simulator" | "device";
  udid: string;
  name?: string;
};

/**
 * Ask user to select one of the Booted/Shutdown simulators
 */
export async function askSimulator(
  asker: UserAsker,
  destinationsManager: DestinationsManager,
  options: {
    title: string;
    state: "Booted" | "Shutdown";
    error: string;
  },
): Promise<SimulatorDestination> {
  let simulators = await destinationsManager.getSimulators({
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

  const selected = await asker.pick({
    title: options.title,
    items: simulators.map((simulator) => ({
      label: simulator.label,
      context: { simulator: simulator },
    })),
  });

  return selected.context.simulator;
}

export type AskBuildContextDeps = XcodeCliDeps & {
  asker: UserAsker;
  progress: ProgressReporter;
};

/**
 * Ask user to select simulator or device to run on
 */
export async function askDestinationToRunOn(
  deps: AskBuildContextDeps,
  destinationsManager: DestinationsManager,
  options: {
    scheme: string;
    configuration: string;
    sdk: string | undefined;
    xcworkspace: string;
  },
): Promise<Destination> {
  deps.progress.updateText("Searching for destinations");
  const destinations = await destinationsManager.getDestinations({
    mostUsedSort: true,
  });

  const cachedDestination = destinationsManager.getSelectedXcodeDestinationForBuild();
  if (cachedDestination) {
    const destination = destinations.find((d) => d.id === cachedDestination.id && d.type === cachedDestination.type);
    if (destination) {
      return destination;
    }
  }

  // We can remove platforms that are not supported by the build settings.
  // WARNING: to avoid refetching build settings, move this logic into the build manager later.
  const buildSettings = await getBuildSettingsToAskDestination(deps, {
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: options.sdk,
    xcworkspace: options.xcworkspace,
  });
  const supportedPlatforms = buildSettings?.supportedPlatforms;

  return await selectDestinationForBuild(deps.asker, destinationsManager, {
    destinations: destinations,
    supportedPlatforms: supportedPlatforms,
  });
}

export async function selectDestinationForBuild(
  asker: UserAsker,
  destinationsManager: DestinationsManager,
  options: {
    destinations: Destination[];
    supportedPlatforms: DestinationPlatform[] | undefined;
  },
): Promise<Destination> {
  const { supported, unsupported } = splitSupportedDestinatinos({
    destinations: options.destinations,
    supportedPlatforms: options.supportedPlatforms,
  });

  type DestRow = {
    label: string;
    iconId: string;
    detail: string | undefined;
    context: Destination;
  };
  type DestSep = { kind: "separator"; label: string };
  type DestItem = DestRow | DestSep;
  const supportedRows: DestRow[] = supported.map((destination) => ({
    label: destination.name,
    iconId: destination.icon,
    detail: destination.quickPickDetails,
    context: destination,
  }));
  const unsupportedRows: DestRow[] = unsupported.map((destination) => ({
    label: destination.name,
    iconId: destination.icon,
    detail: destination.quickPickDetails,
    context: destination,
  }));

  const items: DestItem[] = [];
  if (unsupported.length === 0 && supported.length === 0) {
    items.push({ kind: "separator", label: "No destinations found" });
  } else if (supported.length > 0 && unsupported.length > 0) {
    items.push({ kind: "separator", label: "Supported platforms" });
    items.push(...supportedRows);
    items.push({ kind: "separator", label: "Other" });
    items.push(...unsupportedRows);
  } else {
    items.push(...supportedRows);
    items.push(...unsupportedRows);
  }

  const selected = await asker.pick<Destination>({
    title: "Select destination to run on",
    items: items,
  });

  const destination = selected.context;

  destinationsManager.setWorkspaceDestinationForBuild(destination);
  return destination;
}

/**
 * Ask user to select scheme to build
 */
export async function askSchemeForBuild(
  deps: AskBuildContextDeps,
  buildManager: BuildManager,
  options: {
    title?: string;
    xcworkspace: string;
    ignoreCache?: boolean;
  },
): Promise<string> {
  deps.progress.updateText("Searching for scheme");

  const cachedScheme = buildManager.getDefaultSchemeForBuild();
  if (cachedScheme && !options.ignoreCache) {
    return cachedScheme;
  }

  const schemes = await getSchemes(deps, {
    xcworkspace: options.xcworkspace,
  });

  const scheme = await deps.asker.pick({
    title: options?.title ?? "Select scheme to build",
    items: schemes.map((s) => ({
      label: s.name,
      context: { scheme: s },
    })),
  });

  const schemeName = scheme.context.scheme.name;
  buildManager.setDefaultSchemeForBuild(schemeName);
  return schemeName;
}

/**
 * Prepare storage path for the engine. Caller ensures it exists.
 */
export async function ensureStoragePath(storagePath: string): Promise<string> {
  await createDirectory(storagePath);
  return storagePath;
}

/**
 * Prepare bundle directory for the given scheme in the storage path
 */
export async function prepareBundleDir(storagePath: string, scheme: string): Promise<string> {
  await ensureStoragePath(storagePath);

  const bundleDir = path.join(storagePath, "bundle", scheme);

  await removeDirectory(bundleDir);

  // Remove old .xcresult if exists
  const xcresult = path.join(storagePath, "bundle", `${scheme}.xcresult`);
  await removeDirectory(xcresult);

  return bundleDir;
}

export function prepareDerivedDataPath(deps: { config: ConfigProvider; cwd: string }): string | null {
  const configPath = deps.config.get("build.derivedDataPath");

  if (!configPath) {
    return null;
  }

  let derivedDataPath: string = configPath;
  if (!path.isAbsolute(configPath)) {
    derivedDataPath = path.join(deps.cwd, configPath);
  }

  return derivedDataPath;
}

export function getCurrentXcodeWorkspacePath(deps: {
  config: ConfigProvider;
  state: WorkspaceState;
  cwd: string;
}): string | undefined {
  const configPath = deps.config.get("build.xcodeWorkspacePath");
  if (configPath) {
    deps.state.update("build.xcodeWorkspacePath", undefined);
    if (path.isAbsolute(configPath)) {
      return configPath;
    }
    return path.join(deps.cwd, configPath);
  }

  const cachedPath = deps.state.get("build.xcodeWorkspacePath");
  if (cachedPath) {
    return cachedPath;
  }

  return undefined;
}

export type WorkspacePathDeps = XcodeCliDeps & {
  state: WorkspaceState;
  asker: UserAsker;
};

export async function askXcodeWorkspacePath(deps: WorkspacePathDeps, buildManager: BuildManager): Promise<string> {
  const current = getCurrentXcodeWorkspacePath({ config: deps.config, state: deps.state, cwd: deps.cwd });
  if (current) {
    return current;
  }

  const selectedPath = await selectXcodeWorkspace(deps, { autoselect: true });

  deps.state.update("build.xcodeWorkspacePath", selectedPath);
  void buildManager.refreshSchemes();
  return selectedPath;
}

export async function askConfiguration(
  deps: AskBuildContextDeps,
  buildManager: BuildManager,
  options: { xcworkspace: string },
): Promise<string> {
  deps.progress.updateText("Searching for build configuration");

  const fromConfig = deps.config.get("build.configuration");
  if (fromConfig) {
    return fromConfig;
  }
  const cached = buildManager.getDefaultConfigurationForBuild();
  if (cached) {
    return cached;
  }
  const selected = await askConfigurationBase(deps, {
    xcworkspace: options.xcworkspace,
  });
  buildManager.setDefaultConfigurationForBuild(selected);
  return selected;
}

/**
 * Whether SweetPad should auto-(re)generate `buildServer.json` via xcode-build-server.
 * Reads `build.autoGenerateBuildServerConfig`, falling back to the deprecated
 * `xcodebuildserver.autogenerate` key. Defaults to true.
 */
export function isAutoGenerateBuildServerConfigEnabled(config: ConfigProvider): boolean {
  const current = config.get("build.autoGenerateBuildServerConfig");
  if (current !== undefined) {
    return current;
  }
  return config.get("xcodebuildserver.autogenerate") ?? true;
}

/**
 * Standard regenerate cycle: (re)write buildServer.json and notify the LSP.
 * Used by the manual command, the scheme-change auto-regen, and the on-build
 * auto-regen — keep them in sync.
 */
export async function refreshBuildServer(
  deps: XcodeCliDeps & { lsp: LspRefresher },
  options: {
    xcworkspace: string;
    scheme: string;
    forceRestartLSP?: boolean;
  },
): Promise<void> {
  await generateBuildServerConfig(deps, {
    xcworkspace: options.xcworkspace,
    scheme: options.scheme,
  });
  await deps.lsp.refresh({ force: options.forceRestartLSP });
}

/**
 * Check if buildServer.json needs to be regenerated and regenerate it if needed.
 */
export async function generateBuildServerConfigOnBuild(
  deps: XcodeCliDeps & { lsp: LspRefresher },
  options: { scheme: string; xcworkspace: string },
): Promise<void> {
  if (!isAutoGenerateBuildServerConfigEnabled(deps.config)) {
    return;
  }

  const isServerInstalled = await getIsXcodeBuildServerInstalled(deps);
  if (!isServerInstalled) {
    return;
  }

  let config: XcodeBuildServerConfig | undefined = undefined;
  try {
    config = await readXcodeBuildServerConfig(deps);
  } catch (e) {
    // regenerate config in case of errors like JSON invalid or file does not exist
  }

  // regenerate config only if something is wrong with it:
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
    await refreshBuildServer(deps, {
      xcworkspace: options.xcworkspace,
      scheme: options.scheme,
    });
  }
}

/**
 * Detect xcode workspace in the given directory
 */
export async function detectXcodeWorkspacesPaths(cwd: string): Promise<string[]> {
  return await findFilesRecursive({
    directory: cwd,
    depth: 4,
    matcher: (file) => {
      return file.name.endsWith(".xcworkspace") || file.name === "Package.swift";
    },
  });
}

/**
 * Find xcode workspace in the given directory and ask user to select it
 */
export async function selectXcodeWorkspace(
  deps: XcodeCliDeps & { asker: UserAsker },
  options: { autoselect: boolean },
): Promise<string> {
  const paths = await detectXcodeWorkspacesPaths(deps.cwd);

  if (paths.length === 0) {
    throw new ExtensionError("No xcode workspaces or SPM packages found", {
      context: { cwd: deps.cwd },
    });
  }

  if (paths.length === 1 && options.autoselect) {
    const selectedPath = paths[0];
    const projectType = detectWorkspaceType(selectedPath);
    deps.logger.log("Project was detected", {
      workspace: deps.cwd,
      path: selectedPath,
      projectType: projectType,
    });
    return selectedPath;
  }

  const podfilePath = path.join(deps.cwd, "Podfile");
  const isCocoaProject = await isFileExists(podfilePath);

  const selected = await deps.asker.pick({
    title: "Select Xcode workspace or SPM package",
    items: paths
      .toSorted((a, b) => {
        const aDepth = a.split(path.sep).length;
        const bDepth = b.split(path.sep).length;
        return aDepth - bDepth;
      })
      .map((xwPath) => {
        const relativePath = path.relative(deps.cwd, xwPath);
        const parentDir = path.dirname(relativePath);

        const isInRootDir = parentDir === ".";
        const isCocoaPods = isInRootDir && isCocoaProject;
        const isSPMPackage = detectWorkspaceType(xwPath) === "spm";

        let detail: string | undefined;
        if (isSPMPackage) {
          detail = "Swift Package Manager";
        } else if (isCocoaPods && isInRootDir) {
          detail = "CocoaPods (recommended)";
        } else if (!isInRootDir && parentDir.endsWith(".xcodeproj")) {
          detail = "Xcode";
        }

        return {
          label: relativePath,
          detail: detail,
          context: { path: xwPath },
        };
      }),
  });
  return selected.context.path;
}

export function isXcbeautifyEnabled(config: ConfigProvider): boolean {
  return config.get("build.xcbeautifyEnabled") ?? true;
}

export class XcodeCommandBuilder {
  NO_VALUE = "__NO_VALUE__";

  private xcodebuild: string;
  private parameters: {
    arg: string;
    value: string | "__NO_VALUE__";
  }[] = [];
  private buildSettings: { key: string; value: string }[] = [];
  private actions: string[] = [];
  private logger: Logger;

  constructor(deps: XcodeCliDeps) {
    this.xcodebuild = getXcodeBuildCommand(deps);
    this.logger = deps.logger;
  }

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
        this.logger.warn("Unknown argument", {
          argument: current,
          args: args,
        });
      }
    }

    // Remove duplicates, with higher priority for the last occurrence
    const seenParameters = new Set<string>();
    this.parameters = this.parameters
      .slice()
      .toReversed()
      .filter((param) => {
        if (seenParameters.has(param.arg)) {
          return false;
        }
        seenParameters.add(param.arg);
        return true;
      })
      .toReversed();

    const seenActions = new Set<string>();
    this.actions = this.actions.filter((action) => {
      if (seenActions.has(action)) {
        return false;
      }
      seenActions.add(action);
      return true;
    });

    const seenSettings = new Set<string>();
    this.buildSettings = this.buildSettings
      .slice()
      .toReversed()
      .filter((setting) => {
        if (seenSettings.has(setting.key)) {
          return false;
        }
        seenSettings.add(setting.key);
        return true;
      })
      .toReversed();
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
export function getXcodeBuildDestinationString(options: { destination: Destination; config: ConfigProvider }): string {
  const destination = options.destination;
  const arch = getSimulatorArch(options.config);

  if (destination.type === "iOSSimulator") {
    return buildDestinationString({ platform: "iOS Simulator", id: destination.udid, arch });
  }
  if (destination.type === "watchOSSimulator") {
    return buildDestinationString({ platform: "watchOS Simulator", id: destination.udid, arch });
  }
  if (destination.type === "tvOSSimulator") {
    return buildDestinationString({ platform: "tvOS Simulator", id: destination.udid, arch });
  }
  if (destination.type === "visionOSSimulator") {
    return buildDestinationString({ platform: "visionOS Simulator", id: destination.udid, arch });
  }
  if (destination.type === "macOS") {
    // note: without arch, xcodebuild will warn about multiple matching destinations
    // when both arm64 and x86_64 slices are available on the same Mac.
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
function buildDestinationString(options: { platform: string; id?: string; arch?: string }): string {
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
  return `platform=${platform}`;
}

function getSimulatorArch(config: ConfigProvider): string | undefined {
  // Rosetta is technology that allows running x86_64 code on Apple Silicon Macs.
  // This function instructs xcodebuild to build for x86_64 architecture when
  // Rosetta destinations are enabled in Xcode.
  const useRosetta = config.get("build.rosettaDestination") ?? false;
  if (useRosetta) {
    return "x86_64";
  }
  return undefined;
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

export type GitWorktree = {
  path: string;
  branch: string;
};

/**
 * Detect git worktrees by running `git worktree list --porcelain`.
 * Returns an array of worktrees with their paths and branch names.
 *
 * Example output of `git worktree list --porcelain`:
 *
 *   worktree /Users/user/project
 *   HEAD abc1234
 *   branch refs/heads/main
 *
 *   worktree /Users/user/project-feature
 *   HEAD def5678
 *   branch refs/heads/feature/login
 *
 *   worktree /Users/user/project-detached
 *   HEAD 9876543
 *   detached
 */
export async function detectGitWorktrees(deps: { cwd: string; logger: Logger }): Promise<GitWorktree[]> {
  let output: string;
  try {
    // Use execa directly to avoid relying on the engine's shell-env warmup for a simple git call.
    const result = await execa("git", ["worktree", "list", "--porcelain"], {
      cwd: deps.cwd,
    });
    output = result.stdout;
  } catch {
    deps.logger.warn("Failed to list git worktrees — git may not be available or this is not a git repo");
    return [];
  }

  const worktrees: GitWorktree[] = [];
  const blocks = output.trim().split("\n\n");

  for (const block of blocks) {
    const lines = block.trim().split("\n");
    let worktreePath = "";
    let branch = "";
    let isBare = false;

    for (const line of lines) {
      if (line.startsWith("worktree ")) {
        worktreePath = line.substring("worktree ".length);
      } else if (line.startsWith("branch ")) {
        const refPath = line.substring("branch ".length);
        branch = refPath.replace("refs/heads/", "");
      } else if (line === "bare") {
        isBare = true;
      } else if (line === "detached") {
        branch = "(detached HEAD)";
      }
    }

    if (worktreePath && !isBare) {
      worktrees.push({ path: worktreePath, branch });
    }
  }

  return worktrees;
}

/**
 * Find Xcode workspace/project or SPM package files inside a given directory (up to 4 levels).
 * Returns the first .xcworkspace or Package.swift path found, or undefined.
 */
export async function findXcodeWorkspaceInDirectory(directory: string): Promise<string | undefined> {
  const paths = await findFilesRecursive({
    directory,
    depth: 4,
    ignore: ["Pods", "DerivedData", ".build", "node_modules"],
    maxResults: 1,
    matcher: (file) => file.name.endsWith(".xcworkspace") || file.name === "Package.swift",
  });
  return paths.length > 0 ? paths[0] : undefined;
}

/**
 * Translate a scheme's <LaunchAction> into launch-time argv + env, mirroring
 * what Xcode itself injects when you press Run:
 *
 *  - enabled <CommandLineArgument>s are appended (whitespace-split, the way
 *    Xcode tokenizes each row so that `-AppleLanguages (he)` becomes two argv)
 *  - enabled <EnvironmentVariable>s become entries in `env`
 *  - the `language` / `region` attrs (Edit Scheme → Options → App Language /
 *    App Region) become argv flags Foundation reads at launch via
 *    NSArgumentDomain:
 *      - `language` alone     → `-AppleLanguages (<lang>)`
 *      - `language + region`  → `-AppleLanguages (<lang>) -AppleLocale <lang>_<region>`
 *      - `region` alone       → nothing (a bare region code like "IL" is not
 *                                a valid POSIX locale identifier; Xcode would
 *                                pair it with the device's system language,
 *                                which we can't know here. The user can add
 *                                an explicit `-AppleLocale` CLI arg if needed.)
 *    Discussion #197 use case.
 */
export function launchActionToSettings(action: LaunchAction): {
  args: string[];
  env: Record<string, string>;
} {
  const args: string[] = [];

  for (const arg of action.commandLineArguments()) {
    if (arg.isEnabled === false) continue;
    const raw = arg.argument;
    if (!raw) continue;
    // Xcode splits each row on whitespace (no shell-style quote handling), so
    // `-AppleLanguages (he)` becomes `["-AppleLanguages", "(he)"]`.
    for (const token of raw.trim().split(/\s+/)) {
      if (token) args.push(token);
    }
  }

  const language = action.language;
  const region = action.region;
  if (language) {
    args.push("-AppleLanguages", `(${language})`);
  }
  if (language && region) {
    args.push("-AppleLocale", `${language}_${region}`);
  }

  const env: Record<string, string> = {};
  for (const ev of action.environmentVariables()) {
    if (ev.isEnabled === false) continue;
    const key = ev.key;
    const value = ev.value;
    if (key && value !== undefined) {
      env[key] = value;
    }
  }

  return { args, env };
}

/**
 * Load the scheme file for `scheme` inside `xcworkspace` and extract its
 * <LaunchAction> args/env. Returns empty values if the scheme can't be found,
 * has no on-disk file (default scheme), or has no <LaunchAction>.
 */
export async function getSchemeLaunchSettings(
  deps: { logger: Logger },
  options: {
    xcworkspace: string;
    scheme: string;
  },
): Promise<{ args: string[]; env: Record<string, string> }> {
  try {
    const workspace = await XcodeWorkspace.parseWorkspace(options.xcworkspace, { logger: deps.logger });
    const scheme = await workspace.getScheme({ name: options.scheme });
    const doc = await scheme?.getScheme();
    const action = doc?.launchAction();
    if (!action) {
      return { args: [], env: {} };
    }
    return launchActionToSettings(action);
  } catch (error) {
    deps.logger.warn("Failed to read scheme launch settings; continuing without them", {
      error: error,
      scheme: options.scheme,
      xcworkspace: options.xcworkspace,
    });
    return { args: [], env: {} };
  }
}
