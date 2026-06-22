import { promises as fs } from "node:fs";
import * as path from "node:path";

import * as vscode from "vscode";

import { getWorkspacePath } from "../build/utils";
import { getDeveloperDir, getIsNodeInstalled, getIsXBSInstalled } from "../common/cli/scripts";
import { type AppDeps, NODE_DOWNLOAD_URL } from "../common/commands";
import { getWorkspaceConfig, updateWorkspaceConfig } from "../common/config";
import { isFileExists } from "../common/files";
import { assertUnreachable } from "../common/types";

type DoctorCheck = {
  ok: boolean;
  label: string;
  detail?: string;
  hint?: string;
};

const SWIFT_RESTART_COMMAND = "swift.restartLSPServer";

export function getBuildServerProvider(): "sweetpad" | "xcode-build-server" {
  return getWorkspaceConfig("buildServer.provider") ?? "xcode-build-server";
}

export async function isSweetpadBuildServerActive(workspacePath: string): Promise<boolean> {
  if (getBuildServerProvider() !== "sweetpad") return false;
  return isFileExists(path.join(workspacePath, "buildServer.json"));
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

  checks.push({ ok: true, label: "Build server provider", detail: "xcode-build-server" });

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

async function collectSweetpadChecks(deps: AppDeps): Promise<DoctorCheck[]> {
  const checks: DoctorCheck[] = [];
  const snap = deps.bspService.snapshot();

  checks.push({ ok: true, label: "Build server provider", detail: "sweetpad" });

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
