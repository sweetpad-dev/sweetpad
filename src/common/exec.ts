import { getWorkspacePath } from "../build/utils.js";
import { ExecBaseError, ExecErrror } from "./errors.js";
import { commonLogger } from "./logger.js";

export type ExecaError = {
  command: string;
  escapedCommand: string;
  exitCode: number;
  stdout: string;
  stderr: string;
  failed: boolean;
  timedOut: boolean;
  killed: boolean;
  signal?: string;
  signalDescription?: string;
  cwd: string;
  message: string;
  shortMessage: string;
  originalMessage: string;
};

/**
 * Execa is ESM only, so we need to import it dynamically, because we are using CJS and
 * can't move to ESM yet due to vscode limitations
 */
async function getExeca() {
  return await import("execa");
}

export async function preloadExec() {
  await getExeca();
}

export async function exec(options: { command: string; args: string[]; cwd?: string }): Promise<string> {
  const execa = await getExeca();
  const cwd = options.cwd ?? getWorkspacePath();

  let result;
  try {
    result = await execa.execa(options.command, options.args, {
      cwd: cwd,
    });
  } catch (e: any) {
    const errorMessage: string = e?.shortMessage ?? e?.message ?? "[unknown error]";
    const stderr: string | undefined = e?.stderr;
    throw new ExecBaseError(`Error executing "${options.command}" command`, {
      errorMessage: errorMessage,
      stderr: stderr,
      command: options.command,
      args: options.args,
      cwd: cwd,
    });
  }

  if (result.stdout && result.stderr) {
    commonLogger.warn(`Both stdout and stderr are not empty for "${options.command}" command`, {
      stdout: result.stdout,
      stderr: result.stderr,
      command: options.command,
      args: options.args,
      cwd: cwd,
    });
    return result.stdout;
  }

  if (result.stderr) {
    throw new ExecErrror(`Error executing "${options.command}" command`, {
      stderr: result.stderr,
      command: options.command,
      args: options.args,
      cwd: cwd,
      exitCode: result.exitCode,
      errorMessage: "[stderr not empty]",
    });
  }

  return result.stdout;
}
