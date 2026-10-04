import { accessSync, constants, statSync } from "node:fs";
import os from "node:os";
import path from "node:path";

import { getSweetpadCliPath } from "../common/cli/scripts";
import { getWorkspaceConfig } from "../common/config";
import { exec } from "../common/exec";
import { commonLogger } from "../common/logger";

/**
 * Where Homebrew links the CLI: Apple silicon first, then Intel. A VS Code started from the
 * Dock can carry a PATH that holds neither, and the login shell may not either when Homebrew's
 * `shellenv` line is missing from the dotfiles.
 */
export const HOMEBREW_CLI_PATHS: readonly string[] = ["/opt/homebrew/bin/sweetpad", "/usr/local/bin/sweetpad"];

/**
 * What debugging can expect from the `sweetpad` CLI.
 *
 * - "ready": found, and it serves `sweetpad dap`.
 * - "no-dap": found, but a release from before `dap`.
 * - "missing": not found. `configuredPath` is set when `sweetpad.debugger.cliPath` names a
 *   file that is not there or not executable.
 */
export type SweetpadCliStatus =
  | { kind: "ready"; path: string }
  | { kind: "no-dap"; path: string }
  | { kind: "missing"; configuredPath: string | undefined };

let cached: { configuredPath: string | undefined; status: Promise<SweetpadCliStatus> } | undefined;

/**
 * The CLI status for the next debug session.
 *
 * A CLI that serves `dap` is remembered until `sweetpad.debugger.cliPath` changes or the file
 * goes away, so a session start costs no spawn. Any other answer is looked up again on the
 * next session, so installing or upgrading the CLI takes effect without a reload.
 */
export async function getSweetpadCliStatus(): Promise<SweetpadCliStatus> {
  const configuredPath = getConfiguredCliPath();
  if (cached && cached.configuredPath === configuredPath) {
    const status = await cached.status;
    if (status.kind === "ready" && isExecutable(status.path)) {
      return status;
    }
  }
  const status = resolveSweetpadCliStatus(configuredPath);
  cached = { configuredPath, status: status };
  return await status;
}

/** Forget the remembered status. For tests. */
export function resetSweetpadCliStatusCache(): void {
  cached = undefined;
}

async function resolveSweetpadCliStatus(configuredPath: string | undefined): Promise<SweetpadCliStatus> {
  const cliPath = await locateSweetpadCli(configuredPath);
  if (cliPath === undefined) {
    commonLogger.debug("SweetPad CLI not found for debugging", { configuredPath });
    return { kind: "missing", configuredPath };
  }
  const status: SweetpadCliStatus = (await supportsDap(cliPath))
    ? { kind: "ready", path: cliPath }
    : { kind: "no-dap", path: cliPath };
  commonLogger.debug("SweetPad CLI status for debugging", { status });
  return status;
}

/**
 * The CLI to debug with: the configured path when one is set, otherwise the login shell's
 * PATH, otherwise Homebrew's link locations.
 */
async function locateSweetpadCli(configuredPath: string | undefined): Promise<string | undefined> {
  if (configuredPath !== undefined) {
    return isExecutable(configuredPath) ? configuredPath : undefined;
  }
  const onPath = await getSweetpadCliPath();
  if (onPath !== undefined && isExecutable(onPath)) {
    return onPath;
  }
  return HOMEBREW_CLI_PATHS.find((candidate) => isExecutable(candidate));
}

/**
 * Whether the CLI has the `dap` command. A release without it exits 2 on `sweetpad dap --help`
 * ("unrecognized subcommand"), which `exec` turns into a throw.
 */
async function supportsDap(cliPath: string): Promise<boolean> {
  try {
    await exec({ command: cliPath, args: ["dap", "--help"], cwd: null });
    return true;
  } catch (error) {
    commonLogger.debug("SweetPad CLI has no dap command", { cliPath, error });
    return false;
  }
}

/** `sweetpad.debugger.cliPath`, with a leading `~` expanded. Undefined when unset or blank. */
function getConfiguredCliPath(): string | undefined {
  const value = getWorkspaceConfig("debugger.cliPath")?.trim();
  if (!value) {
    return undefined;
  }
  if (value === "~" || value.startsWith("~/")) {
    return path.join(os.homedir(), value.slice(1));
  }
  return value;
}

function isExecutable(filePath: string): boolean {
  try {
    accessSync(filePath, constants.X_OK);
    return statSync(filePath).isFile();
  } catch {
    return false;
  }
}
