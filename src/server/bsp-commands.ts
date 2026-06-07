import * as vscode from "vscode";

import type { AppDeps } from "../common/commands";
import { BSP_LOG_LEVELS } from "./bsp-bridge";

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
