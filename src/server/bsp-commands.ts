import { promises as fs } from "node:fs";
import * as path from "node:path";

import * as vscode from "vscode";

import { getWorkspacePath } from "../build/utils";
import { getDeveloperDir, getIsNodeInstalled, getIsXcodeBuildServerInstalled } from "../common/cli/scripts";
import { type AppDeps, NODE_DOWNLOAD_URL } from "../common/commands";
import { BSP_LOG_LEVELS } from "./bsp-bridge";

type DoctorCheck = { ok: boolean; label: string; detail?: string; hint?: string };

const SWIFT_RESTART_COMMAND = "swift.restartLSPServer";
const SETUP_DISMISSED_KEY = "sweetpad.bsp.setup.dismissed";

/** The configured build-server provider backing sourcekit-lsp. */
function getBuildServerProvider(): string {
  return vscode.workspace.getConfiguration("sweetpad").get<string>("buildServer.provider") ?? "xcode-build-server";
}

/**
 * One-click setup: point the build server at sweetpad, enable the control
 * server, and (re)generate buildServer.json. The remaining config is discovered
 * at runtime, so this is all it takes.
 */
export async function bspSetupCommand(): Promise<void> {
  await vscode.workspace
    .getConfiguration("sweetpad")
    .update("buildServer.provider", "sweetpad", vscode.ConfigurationTarget.Workspace);
  await vscode.workspace
    .getConfiguration()
    .update("sweetpad.server.enabled", true, vscode.ConfigurationTarget.Workspace);
  await vscode.commands.executeCommand("sweetpad.build.generateBuildServerConfig");
  void vscode.window.showInformationMessage(
    "SweetPad: Swift code intelligence is set up. Open a Swift file to start the build server.",
  );
}

/**
 * First-run nudge: in an Xcode project that isn't already using sweetpad's build
 * server (and has no buildServer.json yet), offer to set it up. Asks at most
 * once per workspace.
 */
export async function maybeOfferBspSetup(context: vscode.ExtensionContext): Promise<void> {
  if (context.workspaceState.get<boolean>(SETUP_DISMISSED_KEY)) return;
  const provider = vscode.workspace.getConfiguration("sweetpad").get<string>("buildServer.provider");
  if (provider === "sweetpad") return; // already opted in

  try {
    if (await fileExists(path.join(getWorkspacePath(), "buildServer.json"))) return; // a build server is already configured
  } catch {
    return; // no workspace folder
  }

  const xcodeProjects = await vscode.workspace.findFiles("**/*.xcodeproj/project.pbxproj", "**/.build/**", 1);
  if (xcodeProjects.length === 0) return; // not an Xcode project

  const setUp = "Set up";
  const never = "Don't ask again";
  const choice = await vscode.window.showInformationMessage(
    "SweetPad can power Swift code completion and navigation for this Xcode project. Set it up?",
    setUp,
    "Not now",
    never,
  );
  if (choice === setUp) {
    await bspSetupCommand();
  } else if (choice === never) {
    await context.workspaceState.update(SETUP_DISMISSED_KEY, true);
  }
}

/** Reveal the BSP output channel. */
export async function bspShowLogsCommand(deps: AppDeps): Promise<void> {
  if (getBuildServerProvider() !== "sweetpad") {
    const serverEnv =
      vscode.workspace.getConfiguration("sweetpad").get<Record<string, string>>("xcodebuildserver.serverEnv") ?? {};
    const logPath = serverEnv.XBS_LOGPATH;
    void vscode.window.showInformationMessage(
      logPath
        ? `SweetPad: the live BSP log stream is a SweetPad-provider feature. xcode-build-server writes its log to ${logPath} (XBS_LOGPATH).`
        : "SweetPad: the live BSP log stream is a SweetPad-provider feature. To capture xcode-build-server logs, set XBS_LOGPATH in sweetpad.xcodebuildserver.serverEnv.",
    );
    return;
  }
  deps.serverService.revealBspLogs();
}

/** Pick a verbosity for the BSP log stream and push it to connected servers. */
export async function bspSetLogLevelCommand(deps: AppDeps): Promise<void> {
  if (getBuildServerProvider() !== "sweetpad") {
    void vscode.window.showInformationMessage(
      "SweetPad: the BSP log level applies to the SweetPad build-server provider; you're using xcode-build-server.",
    );
    return;
  }
  const current = deps.serverService.getBspLogLevel();
  const picked = await vscode.window.showQuickPick(
    BSP_LOG_LEVELS.map((level) => ({ label: level, description: level === current ? "current" : undefined })),
    { title: "BSP log level", placeHolder: "Verbosity of the BSP log stream" },
  );
  if (!picked) return;
  deps.serverService.setBspLogLevel(picked.label);
  void vscode.window.showInformationMessage(`SweetPad: BSP log level set to "${picked.label}".`);
}

/**
 * Restart the BSP server. Its lifecycle is owned by sourcekit-lsp, so this
 * restarts the Swift language server (which re-spawns the BSP server).
 */
export async function bspRestartCommand(): Promise<void> {
  const available = await vscode.commands.getCommands(true);
  if (!available.includes(SWIFT_RESTART_COMMAND)) {
    void vscode.window.showWarningMessage(
      "SweetPad: Can't restart the BSP server — the Swift extension (which owns sourcekit-lsp) isn't available.",
    );
    return;
  }
  await vscode.commands.executeCommand(SWIFT_RESTART_COMMAND);
  void vscode.window.showInformationMessage("SweetPad: Restarting the Swift language server and the BSP server.");
}

/** Show current BSP/server health, with a shortcut to the logs. */
export async function bspStatusCommand(deps: AppDeps): Promise<void> {
  if (getBuildServerProvider() !== "sweetpad") {
    await xcodeBuildServerStatus();
    return;
  }
  const s = deps.serverService.bspSnapshot();
  const lines = [
    `Server: ${s.serverRunning ? "running" : "stopped"}`,
    `BSP connected: ${s.bspConnected ? "yes" : "no"}`,
    `Phase: ${s.phase}${s.detail ? ` (${s.detail})` : ""}`,
    `Scheme: ${s.scheme ?? "—"}`,
    `Configuration: ${s.configuration ?? "—"}`,
    `Log level: ${s.logLevel}`,
  ];
  const showLogs = "Show logs";
  const choice = await vscode.window.showInformationMessage(
    `SweetPad BSP\n${lines.join("\n")}`,
    { modal: false },
    showLogs,
  );
  if (choice === showLogs) {
    deps.serverService.revealBspLogs();
  }
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
  deps.serverService.writeBspReport(lines);
}

async function fileExists(p: string): Promise<boolean> {
  try {
    await fs.access(p);
    return true;
  } catch {
    return false;
  }
}

/** Read and parse the workspace buildServer.json, or undefined if missing/invalid. */
async function readBuildServerJson(): Promise<Record<string, unknown> | undefined> {
  try {
    const raw = await fs.readFile(path.join(getWorkspacePath(), "buildServer.json"), "utf8");
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return undefined;
  }
}

/** Shared check: buildServer.json exists and carries every field sourcekit-lsp requires. */
async function buildServerJsonCheck(): Promise<DoctorCheck> {
  const config = await readBuildServerJson();
  let ok = false;
  let detail = "not found";
  if (config) {
    const missing = ["name", "version", "bspVersion", "languages", "argv"].filter((k) => config[k] === undefined);
    ok = missing.length === 0;
    detail = ok ? "all required fields present" : `missing: ${missing.join(", ")}`;
  }
  return {
    ok,
    label: "buildServer.json valid",
    detail,
    hint: "Run 'SweetPad: Generate Build Server Config' to (re)create it.",
  };
}

/** Status for the xcode-build-server provider (the SweetPad RPC/BSP server isn't involved). */
async function xcodeBuildServerStatus(): Promise<void> {
  const installed = await getIsXcodeBuildServerInstalled();
  const config = await readBuildServerJson();
  const configState = config ? `present${typeof config.name === "string" ? ` (${config.name})` : ""}` : "missing";
  const lines = [
    "Provider: xcode-build-server",
    `xcode-build-server installed: ${installed ? "yes" : "no"}`,
    `buildServer.json: ${configState}`,
  ];
  const runDoctor = "Run doctor";
  const choice = await vscode.window.showInformationMessage(
    `SweetPad BSP\n${lines.join("\n")}`,
    { modal: false },
    runDoctor,
  );
  if (choice === runDoctor) {
    await vscode.commands.executeCommand("sweetpad.bsp.doctor");
  }
}

async function collectBspChecks(deps: AppDeps): Promise<DoctorCheck[]> {
  return getBuildServerProvider() === "sweetpad" ? collectSweetpadChecks(deps) : collectXcodeBuildServerChecks();
}

/** Diagnose the xcode-build-server provider: its tool, config, and sourcekit-lsp. */
async function collectXcodeBuildServerChecks(): Promise<DoctorCheck[]> {
  const checks: DoctorCheck[] = [];

  checks.push({ ok: true, label: "Build server provider", detail: "xcode-build-server" });

  const installed = await getIsXcodeBuildServerInstalled();
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

/** Diagnose the SweetPad BSP provider: control server, runtime config, and toolchain. */
async function collectSweetpadChecks(deps: AppDeps): Promise<DoctorCheck[]> {
  const checks: DoctorCheck[] = [];
  const snap = deps.serverService.bspSnapshot();

  checks.push({ ok: true, label: "Build server provider", detail: "sweetpad" });

  checks.push({
    ok: vscode.workspace.getConfiguration().get<boolean>("sweetpad.server.enabled") === true,
    label: "Control server enabled (sweetpad.server.enabled)",
    hint: "Enable sweetpad.server.enabled so the BSP server can pull config at runtime.",
  });

  checks.push({
    ok: snap.serverRunning,
    label: "Control server running",
    hint: "Open a Swift/Xcode workspace; the server starts when enabled.",
  });

  checks.push(await buildServerJsonCheck());

  const nodeOk = await getIsNodeInstalled();
  checks.push({
    ok: nodeOk,
    label: "Node.js runtime on PATH",
    detail: nodeOk ? undefined : "node not found",
    hint: `The BSP server launches via "#!/usr/bin/env node"; install Node.js (${NODE_DOWNLOAD_URL}) so it's on your PATH.`,
  });

  checks.push({
    ok: snap.bspConnected,
    label: "BSP server connected to the control channel",
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
