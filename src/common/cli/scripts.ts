import { promises as fs } from "node:fs";
import path from "node:path";

import * as sweetpadLib from "@sweetpad/lib";

import { detectWorkspaceType, getSwiftPMDirectory, getWorkspacePath, prepareDerivedDataPath } from "../../build/utils";
import type { DestinationPlatform } from "../../destination/constants";
import { getWorkspaceConfig } from "../config";
import { ExtensionError } from "../errors";
import { exec } from "../exec";
import { isFileExists, readJsonFile } from "../files";
import { prepareEnvVars } from "../helpers";
import { commonLogger } from "../logger";
import { assertUnreachable } from "../types";

export type SimulatorOutput = {
  dataPath: string;
  dataPathSize: number;
  logPath: string;
  udid: string;
  isAvailable: boolean;
  deviceTypeIdentifier: string;
  state: string;
  name: string;
};

type SimulatorsOutput = {
  devices: { [key: string]: SimulatorOutput[] };
};

export type XcodeScheme = {
  name: string;
};

export type XcodeConfiguration = {
  name: string;
};

/**
 * The build-setting keys `XcodeBuildSettings` actually reads. Passed as a
 * projection to the in-process resolver (`sweetpadLib.buildSettings`) so the
 * launch/destination queries marshal ~10 keys instead of the full ~1.4k-entry
 * map. Raw-map callers (the RPC handlers) intentionally omit this and get every
 * key. The xcodebuild fallback ignores it (xcodebuild has no projection) and
 * returns a superset, which the getters handle.
 */
const XCODE_BUILD_SETTINGS_KEYS = [
  "WRAPPER_NAME",
  "FULL_PRODUCT_NAME",
  "PRODUCT_NAME",
  "TARGET_NAME",
  "EXECUTABLE_PATH",
  "EXECUTABLE_NAME",
  "PRODUCT_BUNDLE_IDENTIFIER",
  "ENABLE_DEBUG_DYLIB",
  "TARGET_BUILD_DIR",
  "SUPPORTED_PLATFORMS",
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

export async function getSimulators(): Promise<SimulatorsOutput> {
  const simulatorsRaw = await exec({
    command: "xcrun",
    args: ["simctl", "list", "--json", "devices"],
  });
  return parseCliJsonOutput<SimulatorsOutput>(simulatorsRaw);
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

  get supportedPlatforms(): DestinationPlatform[] | undefined {
    // ex: ["iphonesimulator", "iphoneos"]
    const platformsRaw = this.settings.SUPPORTED_PLATFORMS; // ex: "iphonesimulator iphoneos"
    if (!platformsRaw) {
      return undefined;
    }
    return platformsRaw.split(" ").map((platform) => {
      return platform as DestinationPlatform;
    });
  }
}

/**
 * Locate a scheme's `.xcscheme` file on disk. Searches the container's own
 * `xcshareddata/xcschemes` and each `xcuserdata/<user>.xcuserdatad/xcschemes`,
 * then — for a workspace — every member project's (via the in-process
 * `listWorkspace`). Returns undefined when the scheme has no file (Xcode's
 * autogenerated default scheme).
 */
export async function findSchemeFile(container: string, scheme: string): Promise<string | undefined> {
  const roots = [container];
  if (container.endsWith(".xcworkspace")) {
    try {
      const ws = sweetpadLib.listWorkspace(container);
      roots.push(...ws.projects);
    } catch {
      // Ignore — fall back to whichever candidate exists (or undefined).
    }
  }

  for (const root of roots) {
    const shared = path.join(root, "xcshareddata", "xcschemes", `${scheme}.xcscheme`);
    if (await isFileExists(shared)) {
      return shared;
    }
    const userScheme = await findUserSchemeFile(root, scheme);
    if (userScheme) {
      return userScheme;
    }
  }
  return undefined;
}

/**
 * Look for `<container>/xcuserdata/<user>.xcuserdatad/xcschemes/<scheme>.xcscheme`
 * across every per-user data directory.
 */
async function findUserSchemeFile(container: string, scheme: string): Promise<string | undefined> {
  const userdataDir = path.join(container, "xcuserdata");
  let entries: string[];
  try {
    entries = await fs.readdir(userdataDir);
  } catch {
    return undefined;
  }
  for (const entry of entries) {
    if (!entry.endsWith(".xcuserdatad")) {
      continue;
    }
    const candidate = path.join(userdataDir, entry, "xcschemes", `${scheme}.xcscheme`);
    if (await isFileExists(candidate)) {
      return candidate;
    }
  }
  return undefined;
}

/**
 * Extract build settings for the given scheme and configuration
 *
 * Pay attention that this function can return an empty array, if the build settings are not available.
 * Also it can return several build settings, if there are several targets assigned to the scheme.
 *
 * `keys` (in-process resolver only) restricts the returned settings to those
 * keys; pass it when you read only a handful (see `XCODE_BUILD_SETTINGS_KEYS`).
 */
export async function getBuildSettingsList(options: {
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
  destination?: string;
  keys?: string[];
}): Promise<XcodeBuildSettings[]> {
  const derivedDataPath = prepareDerivedDataPath();

  const workspaceType = detectWorkspaceType(options.xcworkspace);
  let cwd: string | undefined;

  if (workspaceType === "xcode") {
    const result = sweetpadLib.buildSettings({
      scheme: options.scheme,
      configuration: options.configuration,
      sdk: options.sdk ?? undefined,
      destination: options.destination,
      derivedDataPath: derivedDataPath ?? undefined,
      keys: options.keys,
      ...(options.xcworkspace.endsWith(".xcworkspace")
        ? { workspace: options.xcworkspace }
        : { project: options.xcworkspace }),
    });
    return result.map((entry) => new XcodeBuildSettings({ settings: entry.settings, target: entry.target }));
  }

  // For SPM we still use xcodebuild
  // TODO: consider implementing this in sweetpad-lib as well
  const command = getXcodeBuildCommand();
  const args = [
    "-showBuildSettings",
    "-scheme",
    options.scheme,
    "-configuration",
    options.configuration,
    ...(derivedDataPath ? ["-derivedDataPath", derivedDataPath] : []),
    "-json",
  ];
  if (options.sdk !== undefined) {
    args.push("-sdk", options.sdk);
  }
  if (workspaceType === "spm") {
    cwd = getSwiftPMDirectory(options.xcworkspace);
  } else if (workspaceType === "xcode") {
    args.push("-workspace", options.xcworkspace);
  } else {
    assertUnreachable(workspaceType);
  }

  const stdout = await exec({
    command,
    args,
    cwd,
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
 * Extract build settings for the given scheme and configuration to suggest the destination
 * for the user to select
 */
export async function getBuildSettingsToAskDestination(options: {
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
}): Promise<XcodeBuildSettings | null> {
  try {
    // Only `supportedPlatforms` is read here, so project to the launch keys.
    const settings = await getBuildSettingsList({ ...options, keys: XCODE_BUILD_SETTINGS_KEYS });

    if (settings.length === 0) {
      return null;
    }
    if (settings.length === 1) {
      return settings[0];
    }
    // To ask destination, we might omit the build settings, since they are needed only to
    // to suggest the destination and nothing bad will happen if we don't have them here
    return null;
  } catch (e) {
    commonLogger.error("Error getting build settings", {
      error: e,
    });
    return null;
  }
}

/**
 * Get build settings to launch the app
 *
 * Each scheme might have several targets. That's why -showBuildSettings might return different
 * build settings for each target. In the ideal scenario where there is a single target and settings
 * and I just can use first settings object. But there is a cases when there are several targets
 * for the scheme and I need to find which target is set to launch in .xcscheme XML file.
 */
export async function getBuildSettingsToLaunch(options: {
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
  destination?: string;
}): Promise<XcodeBuildSettings> {
  // Hot launch path: the result feeds only XcodeBuildSettings' getters, so
  // project to those keys.
  const settings = await getBuildSettingsList({ ...options, keys: XCODE_BUILD_SETTINGS_KEYS });

  // Build settings are required to run the app because we use them to locate the executable file or
  // the .app bundle. So let's just give up here if -showBuildSettings didn't return anything.
  if (settings.length === 0) {
    throw new ExtensionError("Empty build settings");
  }

  // I think this is the most common case, when there is only one target in the scheme. Higly likely that
  // this is the target to launch. Technically, scheme mightn't have any target to launch, but I believe
  // this is a rare case.
  if (settings.length === 1) {
    return settings[0];
  }

  // > 1 target in the scheme: pick the one the scheme launches by reading the
  // LaunchAction's runnable via the in-process scheme parser (no workspace-XML
  // parse). findSchemeFile covers both shared and user schemes.
  const schemeFile = await findSchemeFile(options.xcworkspace, options.scheme);
  if (schemeFile) {
    try {
      const launchTarget = sweetpadLib.parseScheme(schemeFile).launchTarget?.blueprintName;
      const targetSettings = settings.find((s) => s.target === launchTarget);
      if (targetSettings) {
        return targetSettings;
      }
    } catch (e) {
      commonLogger.warn("parseScheme failed; using the first resolved target", {
        error: e,
        schemeFile,
      });
    }
  }

  // No on-disk scheme, or its launch target didn't match a resolved target:
  // fall back to the first resolved target.
  return settings[0];
}

/**
 * Find if xcbeautify is installed
 */
export async function getIsXcbeautifyInstalled() {
  try {
    await exec({
      command: "which",
      args: ["xcbeautify"],
    });
    return true;
  } catch (e) {
    return false;
  }
}

/**
 * Get the xcode-build-server command path from config or default
 */
function getXcodeBuildServerCommand(): string {
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
export async function getIsXcodeBuildServerInstalled() {
  const command = getXcodeBuildServerCommand();

  try {
    await exec({
      command: "which",
      args: [command],
    });
    return true;
  } catch (e) {
    return false;
  }
}

/**
 * Is a Node.js runtime on the user's PATH? The bundled CLI (`cli.js`) and the BSP
 * server (`bsp-server.js`) both launch via a `#!/usr/bin/env node` shebang, so
 * they need a real `node` on PATH — VS Code's own bundled Node isn't exposed
 * there. Resolved against the login-shell PATH (via `exec`), the same one the
 * shebang sees.
 */
export async function getIsNodeInstalled(): Promise<boolean> {
  try {
    await exec({ command: "which", args: ["node"] });
    return true;
  } catch (e) {
    return false;
  }
}

export async function getSchemes(options: { xcworkspace: string | undefined }): Promise<XcodeScheme[]> {
  commonLogger.log("Getting schemes", { xcworkspace: options?.xcworkspace ?? "undefined" });

  const workspaceType = detectWorkspaceType(options.xcworkspace ?? "");
  if (workspaceType === "spm") {
    try {
      const packageDir = getSwiftPMDirectory(options.xcworkspace ?? "");
      const stdout = await exec({
        command: getSwiftCommand(),
        args: ["package", "dump-package"],
        cwd: packageDir,
      });
      const packageInfo = JSON.parse(stdout);

      const schemeNames = new Set<string>();

      // Add all library/executable products
      if (packageInfo.products) {
        for (const product of packageInfo.products) {
          if (product.type?.executable || product.type?.library) {
            schemeNames.add(product.name);
          }
        }
      }

      // Add standalone executable targets not already covered
      if (packageInfo.targets) {
        for (const target of packageInfo.targets) {
          if (target.type === "executable" && !schemeNames.has(target.name)) {
            schemeNames.add(target.name);
          }
        }
      }

      // Fallback to the package name if nothing else found
      if (schemeNames.size === 0 && packageInfo.name) {
        schemeNames.add(packageInfo.name);
      }

      return Array.from(schemeNames).map((name) => ({ name }));
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
    return sweetpadLib.schemes(options.xcworkspace).map((name) => ({ name }));
  }
  assertUnreachable(workspaceType);
}

export async function getTargets(options: { xcworkspace: string }): Promise<string[]> {
  const workspaceType = detectWorkspaceType(options.xcworkspace);
  if (workspaceType === "spm") {
    try {
      const packageDir = getSwiftPMDirectory(options.xcworkspace ?? "");
      const stdout = await exec({
        command: getSwiftCommand(),
        args: ["package", "dump-package"],
        cwd: packageDir,
      });
      const packageInfo = JSON.parse(stdout);

      const targets: string[] = [];

      // Add all targets
      if (packageInfo.targets) {
        for (const target of packageInfo.targets) {
          targets.push(target.name);
        }
      }

      return targets;
    } catch (error) {
      commonLogger.error("Failed to get SPM targets", {
        error: error,
        packagePath: options.xcworkspace,
      });
      return [];
    }
  }

  if (workspaceType === "xcode") {
    return sweetpadLib.targets(options.xcworkspace);
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
 * Generate xcode-build-server config.
 *
 * `sweetpad.xcodebuildserver.serverEnv` is injected into the generated
 * `buildServer.json` by prefixing `argv` with `/usr/bin/env KEY=VAL ...` so the
 * long-running build server (later spawned by sourcekit-lsp) inherits them.
 * The vars aren't passed to this short-lived `config` call itself — XBS's
 * config phase only honors them in trivial ways, and the docs flag `argv` as
 * the only stable way to pass env via BSP.
 */
export async function generateBuildServerConfig(options: { xcworkspace: string; scheme: string }) {
  // Opt-in: when the provider is `sweetpad`, always use our own BSP server (it
  // derives compiler args from the project, no build-log parsing). No project-type
  // detection — if you opt in, you get it.
  const provider = getWorkspaceConfig("buildServer.provider") ?? "xcode-build-server";
  if (provider === "sweetpad") {
    await generateSweetpadBuildServerConfig();
    return;
  }

  const workspaceType = detectWorkspaceType(options.xcworkspace);
  const command = getXcodeBuildServerCommand();
  let cwd: string;
  let args: string[];

  if (workspaceType === "spm") {
    cwd = getSwiftPMDirectory(options.xcworkspace);
    args = ["config", "-scheme", options.scheme];
  } else if (workspaceType === "xcode") {
    cwd = getWorkspacePath();
    args = ["config", "-workspace", options.xcworkspace, "-scheme", options.scheme];
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
 * Generate a minimal `buildServer.json` for SweetPad's own BSP server. `argv` is
 * a single entry — the bundled `bsp-server.js`, which carries a
 * `#!/usr/bin/env node` shebang (like the CLI) so sourcekit-lsp execs it directly
 * through the user's Node. No subcommand, so it works the same in any editor
 * (VS Code, Cursor, nvim, Zed).
 *
 * Project, Xcode, scheme, configuration, the log path and the telemetry socket
 * are all read from `.sweetpad/bsp.json`, which the extension writes.
 */
async function generateSweetpadBuildServerConfig(): Promise<void> {
  const cwd = getWorkspacePath();
  // The bundled BSP launcher ships next to the extension bundle (this module's dir).
  const bspServer = path.join(__dirname, "bsp-server.js");
  await ensureExecutable(bspServer);

  // sourcekit-lsp requires all five fields (`name`, `version`, `bspVersion`,
  // `languages`, `argv`) or the decode throws and the server is silently skipped.
  const config = {
    name: "sweetpad",
    version: "0.1.0",
    bspVersion: "2.2.0",
    languages: ["swift", "objective-c", "objective-cpp", "c", "cpp"],
    argv: [bspServer],
  };
  await fs.writeFile(path.join(cwd, "buildServer.json"), `${JSON.stringify(config, null, 2)}\n`, "utf8");
}

/**
 * Make the bundled BSP launcher executable. Its `#!/usr/bin/env node` shebang
 * lets sourcekit-lsp exec the `.js` directly, but only if the file keeps its
 * executable bit — a VSIX zip can drop it. Best-effort chmod; a Node script needs
 * no de-quarantine or code-signing (only a bare Mach-O hits amfi on launch).
 */
async function ensureExecutable(bspServer: string): Promise<void> {
  try {
    await fs.chmod(bspServer, 0o755);
  } catch (e) {
    commonLogger.debug("Failed to chmod the BSP launcher", { error: e, bspServer });
  }
}

/** The active Xcode developer dir (`DEVELOPER_DIR`, else `xcode-select -p`). */
export async function getDeveloperDir(): Promise<string | undefined> {
  if (process.env.DEVELOPER_DIR) {
    return process.env.DEVELOPER_DIR;
  }
  try {
    return (await exec({ command: "xcode-select", args: ["-p"] })).trim();
  } catch {
    return undefined;
  }
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

export type XcodeBuildServerConfig = {
  scheme: string;
  workspace: string;
  build_root: string;
  // there might be more properties, like "kind", "args", "name", but we don't need them for now
};

/**
 * Read xcode-build-server config with proper types
 */
export async function readXcodeBuildServerConfig(): Promise<XcodeBuildServerConfig> {
  const buildServerJsonPath = path.join(getWorkspacePath(), "buildServer.json");
  return await readJsonFile<XcodeBuildServerConfig>(buildServerJsonPath);
}

/**
 * Is XcodeGen installed?
 */
export async function getIsXcodeGenInstalled() {
  try {
    await exec({
      command: "which",
      args: ["xcodegen"],
    });
    return true;
  } catch (e) {
    return false;
  }
}

export async function generateXcodeGen() {
  await exec({
    command: "xcodegen",
    args: ["generate"],
  });
}

export async function getIsTuistInstalled() {
  try {
    await exec({
      command: "which",
      args: ["tuist"],
    });
    return true;
  } catch (e) {
    return false;
  }
}

export async function tuistGenerate() {
  const env = getWorkspaceConfig("tuist.generate.env");
  return await exec({
    command: "tuist",
    args: ["generate", "--no-open"],
    env: env,
  });
}

export async function tuistClean() {
  await exec({
    command: "tuist",
    args: ["clean"],
  });
}

export async function tuistInstall() {
  await exec({
    command: "tuist",
    args: ["install"],
  });
}

export async function tuistEdit() {
  await exec({
    command: "tuist",
    args: ["edit"],
  });
}

export async function tuistTest() {
  await exec({
    command: "tuist",
    args: ["test"],
  });
}

/**
 * Get the Xcode version installed on the system using xcodebuild
 *
 * This version works properly with Xcodes.app, so it's the recommended one
 */
export async function getXcodeVersionInstalled(): Promise<{
  major: number;
}> {
  return { major: sweetpadLib.xcodeVersion().majorVersion };
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
