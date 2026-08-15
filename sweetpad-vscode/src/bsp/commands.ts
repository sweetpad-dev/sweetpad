import { promises as fs, readFileSync } from "node:fs";
import * as path from "node:path";

import * as vscode from "vscode";

import { getWorkspacePath } from "../build/utils";
import {
  SWEETPAD_CLI_MISSING_MESSAGE,
  getDeveloperDir,
  getIsXBSInstalled,
  getSweetpadCliPath,
} from "../common/cli/scripts";
import type { AppDeps } from "../common/commands";
import { getWorkspaceConfig, isWorkspaceConfigSetByUser, updateWorkspaceConfig } from "../common/config";
import { isFileExists, readJsonFile } from "../common/files";
import { assertUnreachable } from "../common/types";
import { getBspConfigFile } from "./paths";

type DoctorCheck = {
  ok: boolean;
  label: string;
  detail?: string;
  hint?: string;
};

const SWIFT_RESTART_COMMAND = "swift.restartLSPServer";

/** The `name` SweetPad writes into a buildServer.json, from either side. */
export const BUILD_SERVER_NAME = "sweetpad";

// Older `sweetpad bsp init` runs wrote the crate the BSP server was first built
// in. Nothing rewrites a config in place, so those files are still out there and
// still ours.
const LEGACY_BUILD_SERVER_NAME = "sweetpad-lib";

const SWEETPAD_BUILD_SERVER_NAMES = new Set<string>([BUILD_SERVER_NAME, LEGACY_BUILD_SERVER_NAME]);

type BuildServerConfig = { name?: unknown; argv?: unknown };

/** Whether SweetPad wrote this config at all, from either side. */
export function isSweetpadBuildServerConfig(config: BuildServerConfig | undefined): boolean {
  return typeof config?.name === "string" && SWEETPAD_BUILD_SERVER_NAMES.has(config.name);
}

/**
 * Whether this config is the one the extension maintains, as opposed to one
 * `sweetpad bsp init` wrote.
 *
 * Told apart by `argv`, not by `name`: both name the same server and both launch
 * the same CLI, and what actually separates them is where the project comes
 * from — ours from the `bsp.json` the extension rewrites as the scheme changes,
 * the CLI's from a `--project`/`--workspace` fixed when it was initialised.
 */
export function isExtensionManagedBuildServerConfig(
  config: BuildServerConfig | undefined,
  workspacePath: string,
): boolean {
  if (!isSweetpadBuildServerConfig(config)) return false;
  if (!Array.isArray(config?.argv)) return false;
  const flag = config.argv.indexOf("--config");
  return flag !== -1 && config.argv[flag + 1] === getBspConfigFile(workspacePath);
}

/**
 * Which build server backs sourcekit-lsp.
 *
 * An explicit setting always wins. Without one, a workspace whose
 * `buildServer.json` SweetPad didn't write keeps whatever owns that file: the
 * built-in server is the default for projects that have yet to choose one, but
 * applying it to a working `xcode-build-server` or hand-maintained setup would
 * replace that setup on the next build without anyone asking. Deleting the
 * file, or running `SweetPad: Set up Swift code intelligence (BSP)`, is what
 * moves such a workspace across.
 */
export function getBuildServerProvider(): "sweetpad" | "xcode-build-server" {
  // Asked via `isWorkspaceConfigSetByUser` rather than by testing the value for
  // undefined: package.json contributes a default, so `getWorkspaceConfig`
  // returns one whether or not anybody chose it, and the evidence below would
  // never be reached.
  if (isWorkspaceConfigSetByUser("buildServer.provider")) {
    const configured = getWorkspaceConfig("buildServer.provider");
    if (configured !== undefined) return configured;
  }
  return hasForeignBuildServerConfig() ? "xcode-build-server" : "sweetpad";
}

/**
 * Whether `buildServer.json` exists and names a server other than ours.
 *
 * Read synchronously and uncached. Every caller arrives here through a user
 * action — a build, a scheme change, a command — rather than a per-file path,
 * and the file is a few hundred bytes; a cache would save nothing measurable
 * and would need invalidating from every place the file can change, including
 * edits made outside VS Code.
 */
function hasForeignBuildServerConfig(): boolean {
  try {
    const raw = readFileSync(path.join(getWorkspacePath(), "buildServer.json"), "utf8");
    return !isSweetpadBuildServerConfig(JSON.parse(raw) as BuildServerConfig);
  } catch {
    // No workspace, no file, unreadable, or not JSON — no evidence of a setup
    // worth keeping. A config too broken to read is one the default repairs
    // rather than one it should defer to.
    return false;
  }
}

/**
 * Whether this workspace runs `xcode-build-server` only because a
 * `buildServer.json` for it is already sitting here, rather than because anyone
 * asked for it.
 */
export function isPreservingForeignBuildServer(): boolean {
  return !isWorkspaceConfigSetByUser("buildServer.provider") && hasForeignBuildServerConfig();
}

export async function isSweetpadBuildServerActive(workspacePath: string): Promise<boolean> {
  if (getBuildServerProvider() !== "sweetpad") return false;
  // sourcekit-lsp launches whatever `buildServer.json` names, so the setting
  // alone doesn't put our server in the loop — the file has to be ours too.
  // Treating any file as proof would have us write `bsp.json` for, and report
  // ourselves active against, a server we aren't running.
  //
  // Narrower than "SweetPad wrote it" on purpose: this gates the `bsp.json` the
  // extension writes and the socket it dials, and a standalone `bsp init` config
  // reads neither. That is our server, but it isn't the one we are driving.
  const config = await readJsonFile<BuildServerConfig>(path.join(workspacePath, "buildServer.json")).catch(
    () => undefined,
  );
  return isExtensionManagedBuildServerConfig(config, workspacePath);
}

export async function bspSetupCommand(): Promise<void> {
  await updateWorkspaceConfig("buildServer.provider", "sweetpad");
  await vscode.commands.executeCommand("sweetpad.build.generateBuildServerConfig");
  void vscode.window.showInformationMessage(
    "SweetPad: BSP setup complete. Open a Swift file to start the build server.",
  );
}

export async function bspShowLogsCommand(deps: AppDeps): Promise<void> {
  const provider = getBuildServerProvider();

  if (provider === "xcode-build-server") {
    const serverEnv = getWorkspaceConfig("xcodebuildserver.serverEnv") ?? {};
    const logPath = serverEnv.XBS_LOGPATH;
    void vscode.window.showInformationMessage(
      logPath
        ? `SweetPad: the live BSP log stream is a SweetPad-provider feature. xcode-build-server writes its log to ${logPath} (XBS_LOGPATH).`
        : "SweetPad: the live BSP log stream is a SweetPad-provider feature. To capture xcode-build-server logs, set XBS_LOGPATH in sweetpad.xcodebuildserver.serverEnv.",
    );
    return;
  }

  if (provider === "sweetpad") {
    deps.bspService.revealLogs();
    return;
  }
  assertUnreachable(provider);
}

/**
 * Run a checklist of the things that make BSP code intelligence work and write
 * the result to the BSP output channel — the answer to "why is autocomplete
 * wrong?". Each failing check carries a fix hint.
 */
export async function bspDoctorCommand(deps: AppDeps): Promise<void> {
  const checks = await collectBspChecks(deps);
  const failed = checks.filter((c) => !c.ok).length;
  const lines = [`SweetPad BSP — diagnosis (${failed === 0 ? "all good" : `${failed} issue(s)`})`, ""];
  for (const c of checks) {
    lines.push(`${c.ok ? "✓" : "✗"} ${c.label}${c.detail ? ` — ${c.detail}` : ""}`);
    if (!c.ok && c.hint) lines.push(`    → ${c.hint}`);
  }
  deps.bspService.writeReport(lines);
}

async function readBuildServerJson(): Promise<Record<string, unknown> | undefined> {
  try {
    const raw = await fs.readFile(path.join(getWorkspacePath(), "buildServer.json"), "utf8");
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return undefined;
  }
}

async function buildServerJsonCheck(): Promise<DoctorCheck> {
  const config = await readBuildServerJson();
  let ok = false;
  let detail = "not found";
  if (config) {
    const missing = ["name", "version", "bspVersion", "languages", "argv"].filter((k) => config[k] === undefined);
    const launcher = Array.isArray(config.argv) ? config.argv[0] : undefined;
    if (missing.length > 0) {
      detail = `missing: ${missing.join(", ")}`;
    } else if (typeof launcher !== "string") {
      detail = "argv is empty";
    } else if (path.isAbsolute(launcher) && !(await isFileExists(launcher))) {
      // Typical after an extension update: argv[0] points into the old
      // (deleted) versioned extension dir, and sourcekit-lsp silently fails
      // to spawn the server.
      detail = `argv[0] does not exist on disk: ${launcher}`;
    } else {
      ok = true;
      detail = "all required fields present";
    }
  }
  return {
    ok,
    label: "buildServer.json valid",
    detail,
    hint: "Run 'SweetPad: Generate Build Server Config' to (re)create it.",
  };
}

async function collectBspChecks(deps: AppDeps): Promise<DoctorCheck[]> {
  return getBuildServerProvider() === "sweetpad" ? collectSweetpadChecks(deps) : collectXBSChecks();
}

async function collectXBSChecks(): Promise<DoctorCheck[]> {
  const checks: DoctorCheck[] = [];

  // Said here rather than in a notification: a workspace kept on
  // xcode-build-server has working code intelligence and no reason to be
  // interrupted, while one that doesn't is being read from this very report.
  checks.push({
    ok: true,
    label: "Build server provider",
    detail: isPreservingForeignBuildServer()
      ? "xcode-build-server — kept because a buildServer.json for it is already here. SweetPad's own server is bundled with the extension; 'SweetPad: Set up Swift code intelligence (BSP)' switches over."
      : "xcode-build-server",
  });

  const installed = await getIsXBSInstalled();
  checks.push({
    ok: installed,
    label: "xcode-build-server installed",
    hint: "Install it with 'brew install xcode-build-server' (or run 'SweetPad: Install tool').",
  });

  checks.push(await buildServerJsonCheck());

  const swiftLspAvailable = (await vscode.commands.getCommands(true)).includes(SWIFT_RESTART_COMMAND);
  checks.push({
    ok: swiftLspAvailable,
    label: "Swift extension (sourcekit-lsp) available",
    hint: "Install the Swift extension so sourcekit-lsp picks up buildServer.json.",
  });

  const developerDir = await getDeveloperDir();
  checks.push({
    ok: developerDir !== undefined,
    label: "Xcode developer dir resolved",
    detail: developerDir,
    hint: "Run 'xcode-select --switch /Applications/Xcode.app' or set DEVELOPER_DIR.",
  });

  return checks;
}

/**
 * A workspace whose `buildServer.json` came from `sweetpad bsp init`. The server
 * is ours, but the CLI binary starts it and is handed the project on the command
 * line, so the extension's socket, its `bsp.json` and its selected scheme never
 * come into it — reporting on those would call a healthy setup broken.
 */
async function collectCliDrivenChecks(config: Record<string, unknown>): Promise<DoctorCheck[]> {
  const argv = config.argv;
  const launcher = Array.isArray(argv) && typeof argv[0] === "string" ? argv[0] : undefined;

  return [
    {
      ok: true,
      label: "Build server provider",
      detail: "sweetpad, launched by the sweetpad CLI — this buildServer.json came from 'sweetpad bsp init'",
    },
    await buildServerJsonCheck(),
    {
      ok: launcher !== undefined && (await isFileExists(launcher)),
      label: "sweetpad CLI present",
      detail: launcher,
      hint: "Reinstall it with 'brew install sweetpad-dev/tap/sweetpad', or run 'SweetPad: Set up Swift code intelligence (BSP)' to use the server bundled with this extension instead.",
    },
    {
      ok: (await vscode.commands.getCommands(true)).includes(SWIFT_RESTART_COMMAND),
      label: "Swift extension (sourcekit-lsp) available",
      hint: "Install the Swift extension so sourcekit-lsp picks up buildServer.json.",
    },
  ];
}

async function collectSweetpadChecks(deps: AppDeps): Promise<DoctorCheck[]> {
  const config = await readBuildServerJson();
  if (isSweetpadBuildServerConfig(config) && !isExtensionManagedBuildServerConfig(config, getWorkspacePath())) {
    return collectCliDrivenChecks(config as Record<string, unknown>);
  }

  const checks: DoctorCheck[] = [];
  const snap = deps.bspService.snapshot();

  checks.push({ ok: true, label: "Build server provider", detail: "sweetpad" });

  checks.push(await buildServerJsonCheck());

  const cli = await getSweetpadCliPath();
  checks.push({
    ok: cli !== undefined,
    label: "sweetpad CLI on PATH",
    detail: cli ?? "not found",
    hint: SWEETPAD_CLI_MISSING_MESSAGE,
  });

  checks.push({
    ok: snap.bspConnected,
    label: "BSP server connected",
    hint: "Open a Swift file so sourcekit-lsp spawns the BSP server, then re-run the doctor.",
  });

  checks.push({
    ok: snap.scheme !== null,
    label: "Scheme selected",
    detail: snap.scheme ?? undefined,
    hint: "Pick a scheme with 'SweetPad: Select scheme for build'.",
  });

  const developerDir = await getDeveloperDir();
  checks.push({
    ok: developerDir !== undefined,
    label: "Xcode developer dir resolved",
    detail: developerDir,
    hint: "Run 'xcode-select --switch /Applications/Xcode.app' or set DEVELOPER_DIR.",
  });

  return checks;
}
