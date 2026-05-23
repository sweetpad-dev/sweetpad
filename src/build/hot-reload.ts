import { existsSync, promises as fs } from "node:fs";
import path from "node:path";

import { getWorkspaceConfig } from "../common/config";
import { exec } from "../common/exec";
import { commonLogger } from "../common/logger";
import type { TaskTerminal } from "../common/tasks/types";
import type { WorkspaceStateService } from "../common/workspace-state";
import type { DestinationType } from "../destination/types";
import { getWorkspacePath } from "./utils";

const INJECT_PACKAGE_URL = "https://github.com/krzysztofzablocki/Inject";
const INJECT_WARNING_MAX = 3;

const INJECTIONNEXT_APP = "/Applications/InjectionNext.app";
const INJECTIONNEXT_RESOURCES = `${INJECTIONNEXT_APP}/Contents/Resources`;

/**
 * Map a destination type to the InjectionNext dylib that ships inside InjectionNext.app.
 * We use the lib*Injection.dylib files rather than the *Injection.bundle directories
 * because DYLD_INSERT_LIBRARIES needs an actual Mach-O path. Returns null for physical
 * devices (codesigning strips DYLD_INSERT_LIBRARIES) and for watchOS (InjectionNext
 * does not ship a watchOS injection dylib).
 */
export function dylibNameFor(type: DestinationType): string | null {
  switch (type) {
    case "iOSSimulator":
      return "libiphonesimulatorInjection.dylib";
    case "visionOSSimulator":
      return "libxrsimulatorInjection.dylib";
    case "tvOSSimulator":
      return "libappletvsimulatorInjection.dylib";
    case "macOS":
      return "libmacosxInjection.dylib";
    case "watchOSSimulator":
    case "iOSDevice":
    case "tvOSDevice":
    case "watchOSDevice":
    case "visionOSDevice":
      return null;
  }
}

/**
 * Xcode's "<Platform>.platform" directory name for a given destination, used to locate
 * XCTest.framework and libXCTestSwiftSupport.dylib that the injection dylib depends on.
 */
export function platformDirNameFor(type: DestinationType): string | null {
  switch (type) {
    case "iOSSimulator":
      return "iPhoneSimulator";
    case "visionOSSimulator":
      return "XRSimulator";
    case "tvOSSimulator":
      return "AppleTVSimulator";
    case "macOS":
      return "MacOSX";
    default:
      return null;
  }
}

let cachedDeveloperDir: string | null | undefined = undefined;

/**
 * Resolve the active Xcode developer dir (the prefix used by xcodebuild and friends),
 * preferring the DEVELOPER_DIR env var if set, otherwise falling back to `xcode-select -p`.
 * Cached for the process lifetime.
 */
async function getXcodeDeveloperDir(): Promise<string | null> {
  if (cachedDeveloperDir !== undefined) return cachedDeveloperDir;
  if (process.env.DEVELOPER_DIR) {
    cachedDeveloperDir = process.env.DEVELOPER_DIR;
    return cachedDeveloperDir;
  }
  try {
    const out = await exec({ command: "xcode-select", args: ["-p"] });
    cachedDeveloperDir = out.trim() || null;
    return cachedDeveloperDir;
  } catch (error) {
    commonLogger.warn("Hot reload: failed to resolve Xcode developer dir via xcode-select", { error: error });
    cachedDeveloperDir = null;
    return null;
  }
}

export function isHotReloadEnabled(): boolean {
  return getWorkspaceConfig("hotReload.enabled") ?? false;
}

/**
 * Whether InjectionNext can hot-reload binaries built for this SDK. Limited to the
 * simulator slices and macOS — physical-device builds strip DYLD_INSERT_LIBRARIES via
 * codesigning, and InjectionNext doesn't ship a watchOS dylib. Used to skip the
 * `-Xlinker -interposable` / EMIT_FRONTEND_COMMAND_LINES build settings on unsupported
 * SDKs so they don't pay for an injection they can never receive.
 */
export function sdkSupportsHotReload(sdk: string): boolean {
  return sdk === "iphonesimulator" || sdk === "appletvsimulator" || sdk === "xrsimulator" || sdk === "macosx";
}

/**
 * Resolve the absolute path to the InjectionNext dylib for a destination type, or null
 * when hot reload is off, the dylib does not exist, or the destination is unsupported.
 */
export function resolveInjectionDylib(destinationType: DestinationType): string | null {
  if (!isHotReloadEnabled()) return null;

  const override = getWorkspaceConfig("hotReload.dylibPath");
  if (override) {
    if (!existsSync(override)) {
      commonLogger.warn("Hot reload: configured dylibPath does not exist", { path: override });
      return null;
    }
    return override;
  }

  const dylibName = dylibNameFor(destinationType);
  if (!dylibName) {
    if (destinationType === "watchOSSimulator") {
      commonLogger.warn("Hot reload: InjectionNext does not ship a watchOS dylib, skipping injection", {
        destination: destinationType,
      });
    } else {
      commonLogger.warn("Hot reload: unsupported destination, skipping injection", {
        destination: destinationType,
      });
    }
    return null;
  }

  const dylibPath = path.join(INJECTIONNEXT_RESOURCES, dylibName);
  if (!existsSync(dylibPath)) {
    commonLogger.warn("Hot reload: InjectionNext.app is not installed", {
      expected: dylibPath,
      hint: "Install via Tools view or download from https://github.com/johnno1962/InjectionNext/releases and drag InjectionNext.app to /Applications.",
    });
    return null;
  }
  return dylibPath;
}

/**
 * Resolve the Platform-specific XCTest search paths for a destination, so dyld can find
 * @rpath/XCTest.framework and @rpath/libXCTestSwiftSupport.dylib that the injection dylib
 * links against. The InjectionNext binaries were built against /Applications/Xcode.app
 * paths that won't exist on machines with a versioned or relocated Xcode install.
 */
async function getXctestSearchPaths(
  destinationType: DestinationType,
): Promise<{ frameworkPath: string; libraryPath: string } | null> {
  const platform = platformDirNameFor(destinationType);
  if (!platform) return null;
  const developerDir = await getXcodeDeveloperDir();
  if (!developerDir) return null;
  const platformDev = path.join(developerDir, "Platforms", `${platform}.platform`, "Developer");
  // XCTest lives in Library/Frameworks but XCTestCore + XCTAutomationSupport are in
  // Library/PrivateFrameworks; we need both on DYLD_FRAMEWORK_PATH for the injection
  // dylib's whole dep graph to resolve.
  return {
    frameworkPath: [
      path.join(platformDev, "Library", "Frameworks"),
      path.join(platformDev, "Library", "PrivateFrameworks"),
    ].join(":"),
    libraryPath: path.join(platformDev, "usr", "lib"),
  };
}

export function prependPath(existing: string | undefined, value: string): string {
  return existing ? `${value}:${existing}` : value;
}

/**
 * Enumerate likely Package.resolved locations for the current workspace:
 *   - <workspace>/Package.resolved                                    (pure SPM root)
 *   - <workspace>/*.xcworkspace/xcshareddata/swiftpm/Package.resolved
 *   - <workspace>/*.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
 */
export async function findPackageResolvedFiles(workspace: string): Promise<string[]> {
  const out: string[] = [];
  const rootResolved = path.join(workspace, "Package.resolved");
  if (existsSync(rootResolved)) out.push(rootResolved);

  let entries: string[] = [];
  try {
    entries = await fs.readdir(workspace);
  } catch {
    return out;
  }

  for (const entry of entries) {
    if (entry.endsWith(".xcworkspace")) {
      const p = path.join(workspace, entry, "xcshareddata", "swiftpm", "Package.resolved");
      if (existsSync(p)) out.push(p);
    } else if (entry.endsWith(".xcodeproj")) {
      const p = path.join(workspace, entry, "project.xcworkspace", "xcshareddata", "swiftpm", "Package.resolved");
      if (existsSync(p)) out.push(p);
    }
  }
  return out;
}

export function pinsContainInject(json: unknown): boolean {
  if (!json || typeof json !== "object") return false;
  const pins = (json as { pins?: unknown }).pins;
  if (!Array.isArray(pins)) return false;
  return pins.some((pin) => {
    if (!pin || typeof pin !== "object") return false;
    const location = (pin as { location?: unknown }).location;
    if (typeof location !== "string") return false;
    return /krzysztofzablocki\/Inject(\.git)?$/i.test(location);
  });
}

/**
 * Print to the run task's terminal if hot reload is on but the project doesn't depend
 * on the Inject package. SwiftUI projects need it to redraw on injection; UIKit-only
 * projects can ignore the warning. Silent when no Package.resolved exists at all
 * (likely a pre-resolve state, or a project that doesn't use SPM yet).
 *
 * Capped at INJECT_WARNING_MAX shows per workspace so it doesn't nag forever for
 * UIKit-only projects that legitimately don't need Inject.
 */
async function warnIfInjectMissing(terminal: TaskTerminal, state: WorkspaceStateService): Promise<void> {
  const shown = state.get("hotReload.injectWarningShownCount") ?? 0;
  if (shown >= INJECT_WARNING_MAX) return;

  let workspace: string;
  try {
    workspace = getWorkspacePath();
  } catch {
    return;
  }

  const files = await findPackageResolvedFiles(workspace);
  if (files.length === 0) return;

  for (const file of files) {
    try {
      const text = await fs.readFile(file, "utf-8");
      if (pinsContainInject(JSON.parse(text))) return;
    } catch {
      // Unreadable / not JSON — skip.
    }
  }

  const remaining = INJECT_WARNING_MAX - shown - 1;
  const suffix = remaining > 0 ? ` (${remaining} reminder${remaining === 1 ? "" : "s"} left)` : " (final reminder)";

  terminal.write(`[sweetpad] Hot reload: \`Inject\` package not found in Package.resolved.${suffix}`, {
    color: "yellow",
    newLine: true,
  });
  terminal.write(
    "[sweetpad] SwiftUI views won't refresh on save until you add Inject as a Swift Package dependency",
    { color: "yellow", newLine: true },
  );
  terminal.write(`[sweetpad]   ${INJECT_PACKAGE_URL}`, { color: "yellow", newLine: true });
  terminal.write(
    "[sweetpad] and annotate views with `@ObserveInjection var inject` + `.enableInjection()`.",
    { color: "yellow", newLine: true },
  );
  terminal.write("[sweetpad] UIKit-only apps can ignore this warning.", {
    color: "yellow",
    newLine: true,
  });

  state.update("hotReload.injectWarningShownCount", shown + 1);
}

/**
 * Return a launchEnv augmented with DYLD_INSERT_LIBRARIES, INJECTION_PROJECT_ROOT, and
 * the DYLD_FRAMEWORK_PATH / DYLD_LIBRARY_PATH needed for the injection dylib's XCTest
 * dependencies to resolve. Pass-through when hot reload is off or unsupported.
 */
export async function withHotReloadLaunchEnv(
  terminal: TaskTerminal,
  state: WorkspaceStateService,
  launchEnv: Record<string, string>,
  destinationType: DestinationType,
): Promise<Record<string, string>> {
  const dylib = resolveInjectionDylib(destinationType);
  if (!dylib) return launchEnv;

  await warnIfInjectMissing(terminal, state);

  const env: Record<string, string> = {
    ...launchEnv,
    DYLD_INSERT_LIBRARIES: dylib,
    INJECTION_PROJECT_ROOT: getWorkspacePath(),
  };

  const xctest = await getXctestSearchPaths(destinationType);
  if (xctest) {
    env.DYLD_FRAMEWORK_PATH = prependPath(launchEnv.DYLD_FRAMEWORK_PATH, xctest.frameworkPath);
    env.DYLD_LIBRARY_PATH = prependPath(launchEnv.DYLD_LIBRARY_PATH, xctest.libraryPath);
  }
  return env;
}

/**
 * Best-effort start of the InjectionNext menu-bar app before launch. Silent if the app
 * isn't installed (the launch path already warned via resolveInjectionDylib).
 */
export async function ensureInjectionAppRunning(): Promise<void> {
  if (!isHotReloadEnabled()) return;
  if (!existsSync(INJECTIONNEXT_APP)) return;
  try {
    await exec({ command: "pgrep", args: ["-x", "InjectionNext"] });
    return;
  } catch {
    // pgrep exits non-zero when no match; fall through to launch.
  }
  try {
    await exec({ command: "open", args: ["-g", "-a", INJECTIONNEXT_APP] });
  } catch (error) {
    commonLogger.warn("Hot reload: failed to auto-start InjectionNext", { error: error });
  }
}
