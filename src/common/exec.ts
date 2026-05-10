import { getWorkspacePath } from "../build/utils";
import { ExecBaseError, ExecError } from "./errors";
import { prepareEnvVars } from "./helpers";
import { commonLogger } from "./logger";
import { getShellEnv } from "./tasks/shell-env";

import { execa } from "execa";

type ExecaError = {
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

export async function exec(options: {
  command: string;
  args: string[];
  cwd?: string;
  env?: { [key: string]: string | null };
}): Promise<string> {
  const cwd = options.cwd ?? getWorkspacePath();

  commonLogger.debug("Executing command", {
    command: options.command,
    args: options.args,
    cwd: cwd,
    env: options.env,
  });

  // Resolve via the user's login+interactive shell so spawned tools (xcbeautify,
  // xcodegen, tuist, mise/asdf shims, …) are found on PATH the same way they are
  // in Terminal. getShellEnv() is cached and warmed at activation; this awaits
  // the warm-up promise if the first exec() lands before it resolves.
  const shellEnv = await getShellEnv();
  const env = { ...shellEnv, ...prepareEnvVars(options.env) };

  let result: any;
  try {
    result = await execa(options.command, options.args, {
      cwd: cwd,
      env: env,
      extendEnv: false,
    });
  } catch (e: any) {
    const errorMessage: string = e?.shortMessage ?? e?.message ?? "[unknown error]";
    const stderr: string | undefined = e?.stderr;
    // todo: imrove logging
    throw new ExecBaseError(`Error executing "${options.command}" command`, {
      errorMessage: errorMessage,
      stderr: stderr,
      command: options.command,
      args: options.args,
      cwd: cwd,
    });
  }

  commonLogger.debug("Command executed", {
    command: options.command,
    args: options.args,
    cwd: cwd,
    stdout: result.stdout,
    stderr: result.stderr,
  });

  // check error code
  if (result.stderr && !result.stdout) {
    throw new ExecError(`Error executing "${options.command}" command`, {
      stderr: result.stderr,
      command: options.command,
      args: options.args,
      cwd: cwd,
      errorMessage: "[stderr not empty]",
    });
  }

  return result.stdout;
}
