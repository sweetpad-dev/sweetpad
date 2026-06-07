import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import * as vscode from "vscode";

import { getIsNodeInstalled } from "../common/cli/scripts";
import { type AppDeps, resetSweetPadState, warnNodeRuntimeMissing } from "../common/commands";
import { isFileExists } from "../common/files";
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
  await refreshShellEnv();
}

const CLI_INSTALL_DEFAULTS = ["/usr/local/bin/sweetpad", path.join(os.homedir(), ".local", "bin", "sweetpad")];

// Symlink not copy so extension upgrades reflect automatically.
export async function installCliCommand(deps: AppDeps): Promise<void> {
  const source = path.join(deps.vscodeContext.extensionPath, "out", "cli.js");
  if (!(await isFileExists(source))) {
    throw new Error(`Bundled CLI not found at ${source}. Was the extension built with npm run build?`);
  }

  try {
    await fs.chmod(source, 0o755);
  } catch (err) {
    commonLogger.warn("Failed to chmod the bundled CLI to 755 — continuing", { error: err });
  }

  const picks = CLI_INSTALL_DEFAULTS.map((value) => ({ label: value, value })) as Array<
    vscode.QuickPickItem & { value: string }
  >;
  picks.push({ label: "Custom path…", description: "type an absolute path", value: "" });
  const selected = await vscode.window.showQuickPick(picks, {
    title: "Install sweetpad CLI",
    placeHolder: "Symlink the bundled CLI to a location on $PATH",
  });
  if (!selected) return;

  let target = selected.value;
  if (!target) {
    const typed = await vscode.window.showInputBox({
      title: "Custom install path",
      placeHolder: "/absolute/path/to/sweetpad",
      validateInput: (v) => (v && path.isAbsolute(v.trim()) ? null : "Enter an absolute path"),
    });
    if (!typed) return;
    target = typed.trim();
  }

  await fs.mkdir(path.dirname(target), { recursive: true });

  const existing = await readLinkOrUndefined(target);
  if (existing !== undefined) {
    const action = await vscode.window.showWarningMessage(
      `A file already exists at ${target}. Replace it?`,
      { modal: true },
      "Replace",
    );
    if (action !== "Replace") return;
    await fs.unlink(target);
  }

  try {
    await fs.symlink(source, target);
  } catch (err) {
    const code = (err as NodeJS.ErrnoException)?.code;
    if (code === "EACCES" || code === "EPERM") {
      throw new Error(
        `Cannot write ${target} (permission denied). Try a user-writable directory like ${CLI_INSTALL_DEFAULTS[1]}, or rerun with privileges.`,
        { cause: err },
      );
    }
    throw err;
  }

  vscode.window.showInformationMessage(`SweetPad CLI installed at ${target}`);

  // The symlinked CLI runs through its `#!/usr/bin/env node` shebang, so it needs
  // a Node runtime on PATH to work at all — point the user at the installer now
  // rather than letting `sweetpad` fail cryptically the first time they run it.
  if (!(await getIsNodeInstalled())) {
    await warnNodeRuntimeMissing("The sweetpad CLI");
  }
}

export async function copyServerNameCommand(deps: AppDeps): Promise<void> {
  const status = deps.serverService.getStatus();
  if (!status.running || !status.name) {
    vscode.window.showWarningMessage("SweetPad server is not running. Enable it via `sweetpad.server.enabled`.");
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
    vscode.window.showWarningMessage("SweetPad server is not running. Enable it via `sweetpad.server.enabled`.");
  }
}

export async function showServerStatusCommand(deps: AppDeps): Promise<void> {
  const status = deps.serverService.getStatus();
  if (!status.running) {
    vscode.window.showInformationMessage("SweetPad server is not running. Enable it via `sweetpad.server.enabled`.");
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

async function readLinkOrUndefined(p: string): Promise<string | undefined> {
  try {
    return await fs.readlink(p);
  } catch (err) {
    const code = (err as NodeJS.ErrnoException)?.code;
    if (code === "ENOENT") return undefined;
    if (code === "EINVAL") {
      // Regular file, not a symlink.
      return p;
    }
    throw err;
  }
}
