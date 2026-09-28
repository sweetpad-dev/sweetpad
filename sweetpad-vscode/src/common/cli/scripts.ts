import { promises as fs } from "node:fs";
import path from "node:path";

import * as sweetpadLib from "@sweetpad/native";

import { getBuildServerProvider } from "../../bsp/commands";
import { getBspConfigFile } from "../../bsp/paths";
import { assembleBspConfig, hasBspConfig, writeBspConfig } from "../../bsp/write";
import {
  XcodeCommandBuilder,
  detectWorkspaceType,
  getSwiftPMDirectory,
  prepareDerivedDataPath,
  xcodeContainerArgs,
} from "../../build/utils";
import type { DestinationPlatform } from "../../destination/constants";
import { getWorkspaceConfig } from "../config";
import { ExtensionError } from "../errors";
import { exec } from "../exec";
import { isFileExists, readJsonFile } from "../files";
import { prepareEnvVars } from "../helpers";
import { commonLogger } from "../logger";
import { getShellDeveloperDir } from "../tasks/shell-env";
import { assertUnreachable } from "../types";

// Injected by rolldown at build time (see rolldown.config.mjs).
declare const GLOBAL_RELEASE_VERSION: string | undefined;

export type XcodeScheme = {
  name: string;
};

export type XcodeConfiguration = {
  name: string;
};

/**
 * The build-setting keys `XcodeBuildSettings` reads for a launch. Passed as a
 * projection to the in-process resolver so the launch queries marshal a few
 * keys instead of the full ~1.4k-entry map; the locator reads its own on top.
 * Raw-map callers (the RPC handlers) omit it and get every key. The xcodebuild
 * route ignores it (xcodebuild has no projection) and returns a superset,
 * which the getters handle.
 */
const LAUNCH_SETTINGS_KEYS = [
  "WRAPPER_NAME",
  "FULL_PRODUCT_NAME",
  "PRODUCT_NAME",
  "TARGET_NAME",
  "EXECUTABLE_PATH",
  "EXECUTABLE_NAME",
  "PRODUCT_BUNDLE_IDENTIFIER",
  "ENABLE_DEBUG_DYLIB",
  "TARGET_BUILD_DIR",
];

export function parseCliJsonOutput<T>(output: string): T {
  try {
    return JSON.parse(output) as T;
  } catch (error1) {
    // Parsing might fail if there are some warnings printed before or after the JSON output
    commonLogger.debug("Output contains invalid JSON, attempting to extract JSON part", {
      output: output,
      error: error1,
    });

    try {
      const startObject = output.indexOf("{");
      const endObject = output.lastIndexOf("}");
      const startArray = output.indexOf("[");
      const endArray = output.lastIndexOf("]");
      const isObjectFound = startObject !== -1 && endObject !== -1;
      const isArrayFound = startArray !== -1 && endArray !== -1;

      if (isObjectFound && (!isArrayFound || startObject < startArray)) {
        const jsonString = output.slice(startObject, endObject + 1);
        return JSON.parse(jsonString) as T;
      }

      if (isArrayFound && (!isObjectFound || startArray < startObject)) {
        const jsonString = output.slice(startArray, endArray + 1);
        return JSON.parse(jsonString) as T;
      }
    } catch (error2) {
      commonLogger.debug("Failed to extract JSON part from output", {
        output: output,
        error: error2,
      });
    }
    throw new ExtensionError("No valid JSON found in CLI output", {
      context: {
        output: output,
        error1: error1,
      },
    });
  }
}

/** Run "simctl list --json devices" and return its output, for the addon's "parseSimulators" to read. */
export async function getSimulatorsJson(): Promise<string> {
  return await exec({
    command: "xcrun",
    args: ["simctl", "list", "--json", "devices"],
    cwd: null,
  });
}

export type BuildSettingsOutput = BuildSettingOutput[];

type BuildSettingOutput = {
  action: string;
  target: string;
  buildSettings: {
    [key: string]: string;
  };
};

export class XcodeBuildSettings {
  public readonly settings: { [key: string]: string };
  public target: string;

  constructor(options: { settings: { [key: string]: string }; target: string }) {
    this.settings = options.settings;
    this.target = options.target;
  }

  private get targetBuildDir() {
    // Example:
    // - /Users/hyzyla/Library/Developer/Xcode/DerivedData/ControlRoom-gdvrildvemgjaiameavxoegdskby/Build/Products/Debug
    return this.settings.TARGET_BUILD_DIR;
  }

  /**
   * Path to the executable file (inside the .app bundle) to be used for running macOS apps
   */
  get executablePath() {
    // Example:
    // - {targetBuildDir}/Control Room.app/Contents/MacOS/Control Room
    return path.join(this.targetBuildDir, this.settings.EXECUTABLE_PATH);
  }

  /**
   * Path to the .app bundle to be used for installation on iOS simulator or device
   */
  get appPath() {
    // Example:
    // - {targetBuildDir}/Control Room.app
    return path.join(this.targetBuildDir, this.appName);
  }

  get appName() {
    // Example:
    // - "Control Room.app"
    if (this.settings.WRAPPER_NAME) {
      return this.settings.WRAPPER_NAME;
    }
    if (this.settings.FULL_PRODUCT_NAME) {
      return this.settings.FULL_PRODUCT_NAME;
    }
    if (this.settings.PRODUCT_NAME) {
      return `${this.settings.PRODUCT_NAME}.app`;
    }
    return `${this.targetName}.app`;
  }

  get executableName() {
    // On iOS this is CFBundleExecutable — the string that appears as the process name in
    // os_log / syslog output. Usually matches PRODUCT_NAME but can diverge (e.g. spaces stripped).
    if (this.settings.EXECUTABLE_NAME) {
      return this.settings.EXECUTABLE_NAME;
    }
    if (this.settings.PRODUCT_NAME) {
      return this.settings.PRODUCT_NAME;
    }
    return this.targetName;
  }

  private get targetName() {
    // Example:
    // - "ControlRoom"
    return this.settings.TARGET_NAME;
  }

  get bundleIdentifier() {
    // Example:
    // - "com.hackingwithswift.ControlRoom"
    return this.settings.PRODUCT_BUNDLE_IDENTIFIER;
  }

  get enableDebugDylib(): boolean {
    // Xcode 15+ Debug Dylib Support: when YES, app code is loaded from
    // <EXECUTABLE>.debug.dylib instead of the main binary.
    return this.settings.ENABLE_DEBUG_DYLIB === "YES";
  }
}

/**
 * The app a build produced, as the shared locator (sweetpad-core's `app_locator`, through the native addon)
 * found it: the target the scheme's Run action launches, or the one that runs on the destination.
 */
export class LaunchableApp {
  constructor(private readonly located: sweetpadLib.LocatedApp) {}

  get target(): string {
    return this.located.target;
  }

  /** The `.app` bundle, to install on a simulator or device. */
  get appPath(): string {
    return this.located.path;
  }

  /** The executable inside the bundle, to run a macOS app. */
  get executablePath(): string {
    return this.located.executable;
  }

  get bundleIdentifier(): string {
    return this.located.bundleId;
  }

  /** The bundle's file name, `Control Room.app`. */
  get appName(): string {
    return path.basename(this.located.path);
  }

  /**
   * CFBundleExecutable: the process name in os_log and syslog output. Usually PRODUCT_NAME, but it can diverge
   * (spaces stripped, for one).
   */
  get executableName(): string {
    return this.located.settings.EXECUTABLE_NAME ?? path.basename(this.located.executable);
  }

  get enableDebugDylib(): boolean {
    // Xcode 15+ Debug Dylib Support: when YES, app code is loaded from
    // <EXECUTABLE>.debug.dylib instead of the main binary.
    return this.located.settings.ENABLE_DEBUG_DYLIB === "YES";
  }
}

/**
 * Locate a scheme's `.xcscheme` file on disk: the one `xcodebuild` reads for the container, from
 * the container itself, its member projects and its local packages, and only the current user's
 * `xcuserdata` (the addon's `locateScheme`). Returns undefined when the scheme has no file (Xcode's
 * autogenerated default scheme).
 */
export async function findSchemeFile(container: string, scheme: string): Promise<string | undefined> {
  return sweetpadLib.locateScheme(container, scheme) ?? undefined;
}

/**
 * Extract build settings for the given scheme and configuration
 *
 * Pay attention that this function can return an empty array, if the build settings are not available.
 * Also it can return several build settings, if there are several targets assigned to the scheme.
 *
 * The settings are the ones the extension's builds resolve: `sweetpad.build.args` reaches both routes, so a
 * `PRODUCT_NAME=`, `-xcconfig` or `-configuration` there changes them as it changes the build.
 *
 * `keys` (in-process resolver only) restricts the returned settings to those
 * keys; pass it when you read only a handful (see `LAUNCH_SETTINGS_KEYS`).
 */
export async function getBuildSettingsList(options: {
  workspaceRoot: string;
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
  destination?: string;
  keys?: string[];
}): Promise<XcodeBuildSettings[]> {
  const workspaceType = detectWorkspaceType(options.xcworkspace);
  if (workspaceType === "xcode") {
    return await resolveXcodeProject({
      ...options,
      inProcess: (native) =>
        sweetpadLib
          .buildSettings(native)
          .map((entry) => new XcodeBuildSettings({ settings: entry.settings, target: entry.target })),
      viaXcodebuild: (settings) => settings,
    });
  }

  // For SPM we still use xcodebuild
  // TODO: consider implementing this in sweetpad-lib as well
  return await getBuildSettingsViaXcodebuild({ ...options, workspaceType });
}

/**
 * Answer a build-settings question about an Xcode project the way `getBuildSettingsList` resolves it: through
 * the in-process resolver, or through xcodebuild when the user customized `build.xcodebuildCommand` (a wrapper
 * that injects env vars, selects a toolchain, …, must serve the read-only queries too, or the settings could
 * disagree with what the wrapper builds) or opted into `sweetpad.system.xcodebuildFallback` and the resolver
 * failed.
 */
async function resolveXcodeProject<T>(options: {
  workspaceRoot: string;
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
  destination?: string;
  keys?: string[];
  inProcess: (native: sweetpadLib.BuildSettingsOptions) => T;
  viaXcodebuild: (settings: XcodeBuildSettings[]) => T;
}): Promise<T> {
  const viaXcodebuild = async () =>
    options.viaXcodebuild(await getBuildSettingsViaXcodebuild({ ...options, workspaceType: "xcode" }));
  if (isXcodeBuildCommandCustomized()) {
    return await viaXcodebuild();
  }
  try {
    return options.inProcess({
      scheme: options.scheme,
      configuration: options.configuration,
      sdk: options.sdk ?? undefined,
      destination: options.destination,
      derivedDataPath: prepareDerivedDataPath({ workspaceRoot: options.workspaceRoot }) ?? undefined,
      // The build runs xcodebuild in the workspace root with these on its command line.
      buildArgs: getWorkspaceConfig("build.args") ?? [],
      workingDirectory: options.workspaceRoot,
      keys: options.keys,
      // Resolve against the login shell's Xcode (a DEVELOPER_DIR exported in
      // dotfiles is invisible to the extension host's own env, which is all
      // the in-process resolver sees). Undefined lets the resolver detect
      // the active Xcode itself.
      xcode: await getShellDeveloperDir(options.workspaceRoot),
      ...(options.xcworkspace.endsWith(".xcworkspace")
        ? { workspace: options.xcworkspace }
        : { project: options.xcworkspace }),
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (getWorkspaceConfig("system.xcodebuildFallback")) {
      commonLogger.warn("In-process build-settings resolver failed; falling back to xcodebuild", {
        error: message,
        scheme: options.scheme,
        xcworkspace: options.xcworkspace,
      });
      return await viaXcodebuild();
    }
    throw new ExtensionError(`Failed to resolve build settings: ${message}`, {
      context: {
        scheme: options.scheme,
        configuration: options.configuration,
        xcworkspace: options.xcworkspace,
        hint: 'Enable "sweetpad.system.xcodebuildFallback" to retry such failures via xcodebuild.',
      },
    });
  }
}

/**
 * Run `xcodebuild -showBuildSettings -json` and parse its output. The only
 * path for SPM packages, the routing target when the user customizes
 * `sweetpad.build.xcodebuildCommand`, and the opt-in safety net
 * (`sweetpad.system.xcodebuildFallback`) when the in-process resolver fails.
 */
async function getBuildSettingsViaXcodebuild(options: {
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  destination?: string;
  xcworkspace: string;
  workspaceType: "xcode" | "spm";
  /** Where to run xcodebuild. Only consulted for Xcode projects — an SPM package names its own. */
  workspaceRoot: string;
}): Promise<XcodeBuildSettings[]> {
  // The builder the builds use, so `sweetpad.build.args` joins the command the way it joins theirs.
  const command = new XcodeCommandBuilder();
  command.addOption("-showBuildSettings");
  command.addParameters("-scheme", options.scheme);
  command.addParameters("-configuration", options.configuration);
  const derivedDataPath = prepareDerivedDataPath({ workspaceRoot: options.workspaceRoot });
  if (derivedDataPath) {
    command.addParameters("-derivedDataPath", derivedDataPath);
  }
  command.addOption("-json");
  if (options.sdk !== undefined) {
    command.addParameters("-sdk", options.sdk);
  }
  if (options.destination !== undefined) {
    command.addParameters("-destination", options.destination);
  }
  let cwd: string | undefined;
  if (options.workspaceType === "spm") {
    cwd = getSwiftPMDirectory(options.xcworkspace);
  } else if (options.workspaceType === "xcode") {
    command.addParameters(...xcodeContainerArgs(options.xcworkspace));
  } else {
    assertUnreachable(options.workspaceType);
  }
  command.addAdditionalArgs(getWorkspaceConfig("build.args") ?? []);
  const [executable, ...args] = command.build();

  const stdout = await exec({
    command: executable,
    args,
    cwd: cwd ?? options.workspaceRoot,
  });

  // Parse the output - first few lines can be invalid json, so we need to skip them
  const lines = stdout.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!line) {
      commonLogger.warn("Empty line in build settings output", {
        stdout: stdout,
        index: i,
      });
      continue;
    }

    if (line.startsWith("{") || line.startsWith("[")) {
      const data = lines.slice(i).join("\n");
      const output = parseCliJsonOutput<BuildSettingsOutput>(data);
      if (output.length === 0) {
        return [];
      }
      return output.map((entry) => {
        return new XcodeBuildSettings({
          settings: entry.buildSettings,
          target: entry.target,
        });
      });
    }
  }
  return [];
}

/**
 * Has the user pointed `sweetpad.build.xcodebuildCommand` at a custom
 * binary/wrapper? Build-settings queries are routed through it (see
 * `getBuildSettingsList`); scheme/target/configuration enumeration always uses
 * the bundled resolver — `notifyCustomXcodebuildReadOnlyScope` tells the user
 * about that split once per workspace.
 */
export function isXcodeBuildCommandCustomized(): boolean {
  return Boolean(getWorkspaceConfig("build.xcodebuildCommand"));
}

/**
 * The platforms the scheme's targets build for, to split the destination picker into supported and other
 * destinations. Read from the targets' authored `SUPPORTED_PLATFORMS` / `SDKROOT` by the CLI's own filter
 * (sweetpad-core's `SupportedPlatforms`), since resolving settings with no destination would bind a default
 * platform instead. Undefined, which filters nothing, when that can't be told: a Swift package, an unreadable
 * project, targets that author neither setting.
 */
export function getSupportedPlatforms(options: {
  scheme: string;
  configuration: string;
  xcworkspace: string;
}): DestinationPlatform[] | undefined {
  try {
    const platforms = sweetpadLib.supportedPlatforms(options.xcworkspace, options.scheme, options.configuration);
    return platforms === null ? undefined : (platforms as DestinationPlatform[]);
  } catch (e) {
    commonLogger.error("Error reading the scheme's supported platforms", {
      error: e,
    });
    return undefined;
  }
}

/**
 * Find the app the last build of the scheme produced, to install and launch it. The build's own arguments
 * (`sweetpad.build.args`) are read the way the build reads them, and the pick among the scheme's targets is the
 * CLI's: the target the scheme's Run action launches, else the app that runs on the destination.
 *
 * A Swift package builds no `.app`: its first target's executable is what runs.
 */
export async function locateBuiltApp(options: {
  workspaceRoot: string;
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
  destination: string | undefined;
}): Promise<LaunchableApp> {
  if (detectWorkspaceType(options.xcworkspace) === "spm") {
    const [first] = await getBuildSettingsList({ ...options, keys: LAUNCH_SETTINGS_KEYS });
    if (!first) {
      throw new ExtensionError("Empty build settings");
    }
    return new LaunchableApp({
      target: first.target,
      path: first.appPath,
      bundleId: first.bundleIdentifier,
      executable: first.executablePath,
      settings: first.settings,
    });
  }
  const located = await resolveXcodeProject({
    ...options,
    keys: LAUNCH_SETTINGS_KEYS,
    inProcess: (native) => sweetpadLib.locateApp(native),
    viaXcodebuild: (settings) =>
      sweetpadLib.pickApp({
        targets: settings.map((entry) => ({ target: entry.target, settings: entry.settings })),
        container: options.xcworkspace,
        scheme: options.scheme,
        destination: options.destination,
        sdk: options.sdk,
        keys: LAUNCH_SETTINGS_KEYS,
      }),
  });
  return new LaunchableApp(located);
}

/**
 * Find if xcbeautify is installed
 */
export async function getIsXcbeautifyInstalled() {
  try {
    await exec({
      command: "which",
      args: ["xcbeautify"],
      cwd: null,
    });
    return true;
  } catch (e) {
    return false;
  }
}

/**
 * Get the xcode-build-server command path from config or default
 */
function getXBSCommand(): string {
  const customPath = getWorkspaceConfig("xcodebuildserver.path");
  return customPath || "xcode-build-server";
}

/**
 * Get the xcodebuild command from config or default
 */
export function getXcodeBuildCommand(): string {
  const customCommand = getWorkspaceConfig("build.xcodebuildCommand");
  return customCommand || "xcodebuild";
}

export function getSwiftCommand(): string {
  const customCommand = getWorkspaceConfig("build.swiftCommand");
  return customCommand || "swift";
}

/**
 * Find if xcode-build-server is installed
 */
export async function getIsXBSInstalled() {
  const command = getXBSCommand();

  try {
    await exec({
      command: "which",
      args: [command],
      cwd: null,
    });
    return true;
  } catch (e) {
    return false;
  }
}

/**
 * The toolchain the addon evaluates manifests with. A manifest is Swift source,
 * so naming what a package declares means running `swift package dump-package`,
 * and the extension host sees neither the login shell's `DEVELOPER_DIR` nor a
 * custom `sweetpad.build.swiftCommand` on its own.
 */
async function manifestToolchain(container: string): Promise<sweetpadLib.ManifestToolchain> {
  return {
    swift: getWorkspaceConfig("build.swiftCommand") || undefined,
    developerDir: await getShellDeveloperDir(path.dirname(container)),
  };
}

export async function getSchemes(options: { xcworkspace: string | undefined }): Promise<XcodeScheme[]> {
  commonLogger.log("Getting schemes", { xcworkspace: options?.xcworkspace ?? "undefined" });

  const workspaceType = detectWorkspaceType(options.xcworkspace ?? "");
  if (workspaceType === "spm") {
    try {
      // What `xcodebuild -list` prints in the package directory, the package's
      // `.swiftpm/xcode` scheme files included. The addon evaluates the
      // manifest without writing into the package.
      const xcworkspace = options.xcworkspace ?? "";
      return (await sweetpadLib.schemes(xcworkspace, await manifestToolchain(xcworkspace))).map((name) => ({
        name,
      }));
    } catch (error) {
      commonLogger.error("Failed to get SPM package info", {
        error,
        packagePath: options.xcworkspace,
      });
      return [];
    }
  }

  if (workspaceType === "xcode") {
    if (!options.xcworkspace) {
      return [];
    }
    // Already merged and sorted in Rust, member packages included. A promise
    // because a workspace with local packages has to evaluate their manifests.
    const toolchain = await manifestToolchain(options.xcworkspace);
    return (await sweetpadLib.schemes(options.xcworkspace, toolchain)).map((name) => ({ name }));
  }
  assertUnreachable(workspaceType);
}

export async function getTargets(options: { xcworkspace: string }): Promise<string[]> {
  const workspaceType = detectWorkspaceType(options.xcworkspace);
  if (workspaceType === "spm") {
    try {
      // Every target the manifest declares, tests included: a target list
      // drives `-only-testing:`, where a test target is the whole point.
      return await sweetpadLib.targets(options.xcworkspace, await manifestToolchain(options.xcworkspace));
    } catch (error) {
      commonLogger.error("Failed to get SPM targets", {
        error: error,
        packagePath: options.xcworkspace,
      });
      return [];
    }
  }

  if (workspaceType === "xcode") {
    // Member projects first, then each local package's targets — merged in
    // Rust, which is why this is a promise.
    return await sweetpadLib.targets(options.xcworkspace, await manifestToolchain(options.xcworkspace));
  }
  assertUnreachable(workspaceType);
}

export async function getBuildConfigurations(options: { xcworkspace: string }): Promise<XcodeConfiguration[]> {
  const workspaceType = detectWorkspaceType(options.xcworkspace);
  if (workspaceType === "spm") {
    // SPM projects typically use Debug and Release configurations
    // TODO: try to extract custom configurations from Package.swift if possible, but for now let's just return the defaults
    return [{ name: "Debug" }, { name: "Release" }];
  }

  if (workspaceType === "xcode") {
    return sweetpadLib.configurations(options.xcworkspace).map((name) => ({ name }));
  }
  assertUnreachable(workspaceType);
}

/**
 * Generate buildServer.json for the current scheme and workspace, based on the configured provider.
 */
export async function generateBuildServerConfig(options: {
  xcworkspace: string;
  scheme: string;
  workspaceRoot: string;
  configuration: string | undefined;
}) {
  const provider = getBuildServerProvider();
  if (provider === "sweetpad") {
    // Swift packages: sourcekit-lsp supports SwiftPM natively, and the sweetpad
    // BSP server only understands Xcode projects. A buildServer.json pointing at
    // it would override that native support and break package semantics, so take
    // the native route — write nothing, and warn about a stale config.
    if (detectWorkspaceType(options.xcworkspace) === "spm") {
      await handleSwiftPackageNativeLsp(options.xcworkspace);
      return;
    }
    await generateSweetpadBuildServerConfig({
      workspaceRoot: options.workspaceRoot,
      xcworkspace: options.xcworkspace,
      scheme: options.scheme,
      configuration: options.configuration,
    });
    return;
  }

  if (provider === "xcode-build-server") {
    await generateXBSBuildServerConfig({
      xcworkspace: options.xcworkspace,
      scheme: options.scheme,
      workspaceRoot: options.workspaceRoot,
    });

    return;
  }

  assertUnreachable(provider);
}

/**
 * Shown wherever the build server can't be set up because the CLI is absent.
 * One string so the wording is the same in an error, a warning and the doctor.
 */
export const SWEETPAD_CLI_MISSING_MESSAGE =
  "SweetPad's build server runs through the sweetpad CLI, which isn't on your PATH. Install it with 'brew install sweetpad-dev/tap/sweetpad', or run 'SweetPad: Install tool'.";

/**
 * Oldest CLI the extension will drive without complaint.
 *
 * The two ship on separate cadences — the extension through the Marketplace,
 * the CLI through Homebrew — so any pair can meet. The floor covers what an
 * older CLI cannot read (the `bsp.json` the extension writes) and equally what
 * it cannot do: 0.1.6 is the first to pass `-resultBundlePath`, and without
 * that Xcode 26 writes no `.xcactivitylog`, leaving the build server with no
 * compiler arguments for any file. Raise it alongside any release that depends
 * on CLI behavior whose absence is silent rather than loud.
 */
export const MINIMUM_SWEETPAD_CLI_VERSION = "0.1.6";

/**
 * What `sweetpad --version` prints, or undefined when it can't be asked.
 */
export async function getSweetpadCliVersion(): Promise<string | undefined> {
  try {
    const output = (await exec({ command: "sweetpad", args: ["--version"], cwd: null })).trim();
    return output.length > 0 ? output : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Absolute path to the `sweetpad` CLI, or undefined when it isn't installed.
 *
 * Resolved to a full path rather than left as a bare name: sourcekit-lsp spawns
 * `argv[0]` itself and doesn't necessarily carry the login shell's `PATH`.
 */
export async function getSweetpadCliPath(): Promise<string | undefined> {
  try {
    const resolved = (await exec({ command: "which", args: ["sweetpad"], cwd: null })).trim();
    return resolved.length > 0 ? resolved : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Generate a minimal `buildServer.json` for SweetPad's own BSP server. `argv`
 * runs `sweetpad bsp serve` with `--config <bsp.json>`, naming the per-project
 * config in the host state dir. Editor-agnostic — works the same in VS Code,
 * Cursor, nvim, Zed.
 *
 * Project, Xcode, scheme, configuration, the log path and the telemetry socket
 * are all read from that `bsp.json`, which is seeded here when absent (see
 * `seedBspConfig`) and kept current by `BspService`.
 *
 * The launcher is the installed CLI rather than anything inside this
 * extension's own directory, so it keeps resolving after an extension update
 * instead of pointing into a version that has been deleted.
 */
export async function generateSweetpadBuildServerConfig(options: {
  workspaceRoot: string;
  xcworkspace: string | undefined;
  scheme: string | undefined;
  configuration: string | undefined;
}): Promise<void> {
  const cwd = options.workspaceRoot;
  const cli = await getSweetpadCliPath();
  if (cli === undefined) {
    throw new ExtensionError(SWEETPAD_CLI_MISSING_MESSAGE);
  }

  await seedBspConfig({
    workspaceRoot: cwd,
    xcworkspace: options.xcworkspace,
    scheme: options.scheme,
    configuration: options.configuration,
  });

  // sourcekit-lsp requires all five fields (`name`, `version`, `bspVersion`,
  // `languages`, `argv`) or the decode throws and the server is silently skipped.
  const config = {
    name: "sweetpad",
    version: GLOBAL_RELEASE_VERSION ?? "0.1.0",
    bspVersion: "2.2.0",
    languages: ["swift", "objective-c", "objective-cpp", "c", "cpp"],
    argv: [cli, "bsp", "serve", "--config", getBspConfigFile(cwd)],
  };
  await fs.writeFile(path.join(cwd, "buildServer.json"), `${JSON.stringify(config, null, 2)}\n`, "utf8");
}

/**
 * Put a `bsp.json` in place before `buildServer.json` names it.
 *
 * `BspService` writes that file only once `buildServer.json` is already here
 * and points at it, so on a first setup neither can go first and the server
 * launches against a `--config` path that does not exist. Seeding here settles
 * the order: the pointer is written second, so it never dangles.
 *
 * Only fills a gap — an existing `bsp.json` holds the live selection.
 *
 * Best-effort, the way `BspService.saveConfig` is: a failure here is logged and
 * the pointer still goes out. Without a `bsp.json` the server falls back to the
 * next write; without a `buildServer.json` there is no setup at all.
 */
async function seedBspConfig(options: {
  workspaceRoot: string;
  xcworkspace: string | undefined;
  scheme: string | undefined;
  configuration: string | undefined;
}): Promise<void> {
  const { workspaceRoot, xcworkspace } = options;
  if (!xcworkspace) {
    return;
  }
  try {
    if (await hasBspConfig(workspaceRoot)) {
      return;
    }
    // A null scheme resolves to the project's default, and a null destination
    // platform leaves shared files to the scheme, until BspService, which sees
    // a buildServer.json of ours only after this call, rewrites it.
    await writeBspConfig(
      assembleBspConfig({
        workspacePath: workspaceRoot,
        xcworkspace: xcworkspace,
        developerDir: (await getDeveloperDir({ workspaceRoot: workspaceRoot })) ?? null,
        scheme: options.scheme ?? null,
        configuration: options.configuration ?? "Debug",
        destinationPlatform: null,
        derivedDataPath: prepareDerivedDataPath({ workspaceRoot: workspaceRoot }) ?? null,
        buildArgs: getWorkspaceConfig("build.args") ?? [],
      }),
    );
  } catch (error) {
    commonLogger.warn("Failed to seed bsp.json", { workspaceRoot: workspaceRoot, error: error });
  }
}

/**
 * The native-sourcekit-lsp route for a Swift package. sourcekit-lsp reads
 * Package.swift and builds the index itself, so we never write a
 * buildServer.json for a package — the sweetpad BSP server only handles Xcode
 * projects, and a config pointing at it would override the native path. A
 * pre-existing buildServer.json (e.g. left over from an Xcode setup) does
 * exactly that, so warn about it. We don't delete it — that's the user's call.
 */
async function handleSwiftPackageNativeLsp(packageManifest: string): Promise<void> {
  const cwd = getSwiftPMDirectory(packageManifest);
  const buildServerJson = path.join(cwd, "buildServer.json");
  if (await isFileExists(buildServerJson)) {
    commonLogger.warn(
      "Swift package: a buildServer.json is present and overrides sourcekit-lsp's native SwiftPM support — remove it so the package resolves correctly",
      { buildServerJson },
    );
  } else {
    commonLogger.log("Swift package: using sourcekit-lsp's native SwiftPM support (no buildServer.json needed)", {
      cwd,
    });
  }
}

async function generateXBSBuildServerConfig(options: {
  xcworkspace: string;
  scheme: string;
  workspaceRoot: string;
}): Promise<void> {
  const workspaceType = detectWorkspaceType(options.xcworkspace);
  const command = getXBSCommand();
  let cwd: string;
  let args: string[];

  if (workspaceType === "spm") {
    cwd = getSwiftPMDirectory(options.xcworkspace);
    args = ["config", "-scheme", options.scheme];
  } else if (workspaceType === "xcode") {
    cwd = options.workspaceRoot;
    args = ["config", ...xcodeContainerArgs(options.xcworkspace), "-scheme", options.scheme];
  } else {
    assertUnreachable(workspaceType);
  }
  await exec({
    command: command,
    args: args,
    cwd: cwd,
  });

  const env = getWorkspaceConfig("xcodebuildserver.serverEnv") ?? {};
  await injectEnvIntoBuildServerConfig(path.join(cwd, "buildServer.json"), env);
}

/**
 * The active Xcode developer dir: `DEVELOPER_DIR` from the login shell (which
 * subsumes the extension host's own env — the probe shell inherits it), else
 * `xcode-select -p`.
 */
export async function getDeveloperDir(options: { workspaceRoot: string }): Promise<string | undefined> {
  const fromShell = await getShellDeveloperDir(options.workspaceRoot);
  if (fromShell) {
    return fromShell;
  }
  try {
    return (await exec({ command: "xcode-select", args: ["-p"], cwd: null })).trim();
  } catch {
    return undefined;
  }
}

/**
 * The simulator GUI bundle of the active Xcode, as an `open -a` argument.
 *
 * Xcode 27 renamed `Simulator.app` to `DeviceHub.app` and moved it up out of
 * the developer dir, so both layouts are probed against the Xcode `simctl`
 * itself will use. An install matching neither falls back to the bare name,
 * which lets LaunchServices resolve it the way it always did.
 */
export async function getSimulatorAppPath(options: { workspaceRoot: string }): Promise<string> {
  const developerDir = await getDeveloperDir(options);
  if (developerDir) {
    const candidates = [
      path.join(developerDir, "Applications", "Simulator.app"),
      path.join(developerDir, "..", "Applications", "DeviceHub.app"),
    ];
    for (const candidate of candidates) {
      if (await isFileExists(candidate)) {
        return path.normalize(candidate);
      }
    }
  }
  return "Simulator";
}

/**
 * Bridge `sweetpad.xcodebuildserver.serverEnv` → the long-running XBS process.
 *
 * sourcekit-lsp reads buildServer.json on project open and execs whatever's in
 * `argv` to be its build server. BSP defines no `env` field — `argv` is the
 * only knob. We use the standard `/usr/bin/env` trick to set vars at exec
 * time:
 *
 *   before:  "argv": ["/opt/homebrew/bin/xcode-build-server"]
 *   after:   "argv": ["/usr/bin/env",
 *                     "XBS_LOGPATH=/tmp/sweetpad-xbs.log",
 *                     "/opt/homebrew/bin/xcode-build-server"]
 *
 * Bails out (no-op) when there's nothing to do: empty env, missing file, or
 * already-wrapped argv. The last case shouldn't happen in practice because
 * `xcode-build-server config` rewrites `argv` from scratch on every call (see
 * upstream config/config.py), but the guard makes this function safe to call
 * twice in a row without an intervening regen.
 */
async function injectEnvIntoBuildServerConfig(
  buildServerJsonPath: string,
  env: { [key: string]: string | null },
): Promise<void> {
  const prepared = prepareEnvVars(env);
  const entries = Object.entries(prepared).filter(([, v]) => v !== undefined) as [string, string][];
  if (entries.length === 0) return;

  let config: { argv?: string[]; [key: string]: unknown };
  try {
    config = await readJsonFile<{ argv?: string[]; [key: string]: unknown }>(buildServerJsonPath);
  } catch (e) {
    commonLogger.debug("buildServer.json not found after generation, skipping env injection", {
      path: buildServerJsonPath,
    });
    return;
  }

  if (!Array.isArray(config.argv) || config.argv.length === 0) return;
  if (config.argv[0] === "/usr/bin/env") return;

  const envArgs = entries.map(([k, v]) => `${k}=${v}`);
  config.argv = ["/usr/bin/env", ...envArgs, ...config.argv];

  await fs.writeFile(buildServerJsonPath, `${JSON.stringify(config, null, 2)}\n`, "utf8");
}

export type XBSConfig = {
  scheme: string;
  workspace: string;
  build_root: string;
  // there might be more properties, like "kind", "args", "name", but we don't need them for now
};

/**
 * Read xcode-build-server config with proper types
 */
export async function readXBSConfig(options: { workspaceRoot: string }): Promise<XBSConfig> {
  const buildServerJsonPath = path.join(options.workspaceRoot, "buildServer.json");
  return await readJsonFile<XBSConfig>(buildServerJsonPath);
}

/**
 * Is XcodeGen installed?
 */
export async function getIsXcodeGenInstalled() {
  try {
    await exec({
      command: "which",
      args: ["xcodegen"],
      cwd: null,
    });
    return true;
  } catch (e) {
    return false;
  }
}

/**
 * `xcodegen generate` reads project.yml from its working directory, so `cwd` names the folder
 * holding the spec. Without it the command runs in the active workspace folder.
 */
export async function generateXcodeGen(options: { cwd: string }) {
  await exec({
    command: "xcodegen",
    args: ["generate"],
    cwd: options.cwd,
  });
}

export async function getIsTuistInstalled() {
  try {
    await exec({
      command: "which",
      args: ["tuist"],
      cwd: null,
    });
    return true;
  } catch (e) {
    return false;
  }
}

/**
 * `tuist generate` reads Project.swift/Workspace.swift from its working directory, so `cwd` names
 * the folder holding them. Without it the command runs in the active workspace folder.
 */
export async function tuistGenerate(options: { cwd: string }) {
  const env = getWorkspaceConfig("tuist.generate.env");
  return await exec({
    command: "tuist",
    args: ["generate", "--no-open"],
    env: env,
    cwd: options.cwd,
  });
}

export async function tuistClean(options: { cwd: string }) {
  await exec({
    command: "tuist",
    args: ["clean"],
    cwd: options.cwd,
  });
}

export async function tuistInstall(options: { cwd: string }) {
  await exec({
    command: "tuist",
    args: ["install"],
    cwd: options.cwd,
  });
}

export async function tuistEdit(options: { cwd: string }) {
  await exec({
    command: "tuist",
    args: ["edit"],
    cwd: options.cwd,
  });
}

export async function tuistTest(options: { cwd: string }) {
  await exec({
    command: "tuist",
    args: ["test"],
    cwd: options.cwd,
  });
}

/**
 * Get the Xcode version installed on the system using xcodebuild
 *
 * This version works properly with Xcodes.app, so it's the recommended one
 */
export async function getXcodeVersionInstalled(options: { workspaceRoot: string }): Promise<{
  major: number;
}> {
  return { major: sweetpadLib.xcodeVersion(await getShellDeveloperDir(options.workspaceRoot)).majorVersion };
}

/**
 * Get the Xcode version installed on the system using pgkutils
 *
 * This version doesn't work properly with Xcodes.app, leave it for reference
 */
export async function getXcodeVersionInstalled_pkgutils(): Promise<{
  major: number;
}> {
  const stdout = await exec({
    command: "pkgutil",
    args: ["--pkg-info=com.apple.pkg.CLTools_Executables"],
    cwd: null,
  });

  /*
  package-id: com.apple.pkg.CLTools_Executables
  version: 15.3.0.0.1.1708646388
  volume: /
  location: /
  install-time: 1718529452
  */
  const versionMatch = stdout.match(/version:\s*(\d+)\./);
  if (!versionMatch) {
    throw new ExtensionError("Error parsing xcode version", {
      context: {
        stdout: stdout,
      },
    });
  }

  const major = Number.parseInt(versionMatch[1]);
  return {
    major,
  };
}
