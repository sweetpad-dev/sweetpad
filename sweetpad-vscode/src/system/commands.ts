import * as sweetpadLib from "@sweetpad/native";
import * as vscode from "vscode";

import { type AppDeps, resetSweetPadState } from "../common/commands";
import { commonLogger } from "../common/logger";
import { refreshShellEnv } from "../common/tasks/shell-env";

export async function resetSweetPadCache(deps: AppDeps) {
  deps.progressStatusBar.updateText("Resetting SweetPad cache");
  resetSweetPadState(deps);
  vscode.window.showInformationMessage("SweetPad cache has been reset");
}

async function createIssue(options: { title: string; body: string; labels: string[] }) {
  const url = new URL("https://github.com/sweetpad-dev/sweetpad/issues/new");
  url.searchParams.append("title", options.title);
  url.searchParams.append("body", options.body);
  url.searchParams.append("labels", options.labels.join(","));

  vscode.env.openExternal(vscode.Uri.parse(url.toString()));
}

export async function createIssueGenericCommand(deps: AppDeps) {
  await createIssue({
    title: "SweetPad issue",
    body: "Please describe your issue here",
    labels: ["bug"],
  });
}

export async function createIssueNoSchemesCommand() {
  const logs = commonLogger.lastFormatted(5);
  const logsBlock = `\`\`\`json\n${logs}\n\`\`\``;
  await createIssue({
    title: "SweetPad issue: No schemes",
    body: `Please describe your issue here.\n\n\nLast logs:\n${logsBlock}`,
    labels: ["bug"],
  });
}

export async function testErrorReportingCommand() {
  commonLogger.log("Testing error reporting", {
    contextKey: "Context value",
  });
  throw new Error("This is a test error");
}

export async function openTerminalPanel() {
  vscode.window.terminals.at(-1)?.show();
}

export async function refreshShellEnvCommand(_deps: AppDeps) {
  // Re-detect the active Xcode too: the in-process resolver memoizes the
  // `xcode-select -p` result for the session, so without this an
  // `xcode-select -s` switch isn't observed until VS Code restarts.
  sweetpadLib.flushXcodeCache();
  await refreshShellEnv();
}

export async function copyServerNameCommand(deps: AppDeps): Promise<void> {
  const status = deps.serverService.getStatus();
  if (!status.running || !status.name) {
    vscode.window.showWarningMessage("SweetPad server is not running. Enable it via `sweetpad.cliServer.enabled`.");
    return;
  }
  await vscode.env.clipboard.writeText(status.name);
  vscode.window.showInformationMessage(`Server name copied: ${status.name}`);
}

export async function restartServerCommand(deps: AppDeps): Promise<void> {
  await deps.serverService.restart();
  const status = deps.serverService.getStatus();
  if (status.running && status.name) {
    vscode.window.showInformationMessage(`SweetPad server restarted: ${status.name}`);
  } else {
    vscode.window.showWarningMessage("SweetPad server is not running. Enable it via `sweetpad.cliServer.enabled`.");
  }
}

export async function showServerStatusCommand(deps: AppDeps): Promise<void> {
  const status = deps.serverService.getStatus();
  if (!status.running) {
    vscode.window.showInformationMessage("SweetPad server is not running. Enable it via `sweetpad.cliServer.enabled`.");
    return;
  }
  const summary = `SweetPad server: ${status.name}\nSocket: ${status.socket}`;
  const action = await vscode.window.showInformationMessage(summary, "Copy name", "Copy socket path");
  if (action === "Copy name" && status.name) {
    await vscode.env.clipboard.writeText(status.name);
  } else if (action === "Copy socket path" && status.socket) {
    await vscode.env.clipboard.writeText(status.socket);
  }
}
