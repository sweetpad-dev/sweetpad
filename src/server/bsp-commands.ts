import { promises as fs } from "node:fs";
import * as path from "node:path";

import * as vscode from "vscode";

import { getWorkspacePath } from "../build/utils";
import { getDeveloperDir } from "../common/cli/scripts";
import type { AppDeps } from "../common/commands";
import { BSP_LOG_LEVELS } from "./bsp-bridge";

type DoctorCheck = { ok: boolean; label: string; detail?: string; hint?: string };

const SWIFT_RESTART_COMMAND = "swift.restartLSPServer";

/** Reveal the BSP output channel. */
export async function bspShowLogsCommand(deps: AppDeps): Promise<void> {
  deps.serverService.revealBspLogs();
}

/** Pick a verbosity for the BSP log stream and push it to connected servers. */
export async function bspSetLogLevelCommand(deps: AppDeps): Promise<void> {
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

async function collectBspChecks(deps: AppDeps): Promise<DoctorCheck[]> {
  const checks: DoctorCheck[] = [];
  const snap = deps.serverService.bspSnapshot();

  const provider = vscode.workspace.getConfiguration("sweetpad").get<string>("buildServer.provider") ?? "xcode-build-server";
  checks.push({
    ok: provider === "sweetpad",
    label: "Build server provider is 'sweetpad'",
    detail: `provider = ${provider}`,
    hint: 'Set "sweetpad.buildServer.provider": "sweetpad", then regenerate buildServer.json.',
  });

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

  let bsOk = false;
  let bsDetail = "not found";
  try {
    const raw = await fs.readFile(path.join(getWorkspacePath(), "buildServer.json"), "utf8");
    const config = JSON.parse(raw) as Record<string, unknown>;
    const missing = ["name", "version", "bspVersion", "languages", "argv"].filter((k) => config[k] === undefined);
    bsOk = missing.length === 0;
    bsDetail = bsOk ? "all required fields present" : `missing: ${missing.join(", ")}`;
  } catch {
    // left as "not found"
  }
  checks.push({
    ok: bsOk,
    label: "buildServer.json valid",
    detail: bsDetail,
    hint: "Run 'SweetPad: Generate Build Server Config' to (re)create it.",
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
