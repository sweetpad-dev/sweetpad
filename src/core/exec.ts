import { execa } from "execa";

import { ExecBaseError, ExecError } from "./errors";
import { prepareEnvVars } from "./helpers";
import type { Logger } from "./logger/types";
import { getShellEnv, type ShellEnvOptions } from "./tasks/shell-env";

export type ExecOptions = {
  command: string;
  args: string[];
  cwd: string;
  logger: Logger;
  env?: { [key: string]: string | null };
  shellEnv?: Omit<ShellEnvOptions, "logger">;
};

export async function exec(options: ExecOptions): Promise<string> {
  const cwd = options.cwd;
  const logger = options.logger;

  logger.debug("Executing command", {
    command: options.command,
    args: options.args,
    cwd: cwd,
    env: options.env,
  });

  // Resolve via the user's login+interactive shell so spawned tools (xcbeautify,
  // xcodegen, tuist, mise/asdf shims, …) are found on PATH the same way they are
  // in Terminal. getShellEnv() is cached and warmed at activation; this awaits
  // the warm-up promise if the first exec() lands before it resolves.
  const shellEnv = await getShellEnv({ ...options.shellEnv, logger: logger });
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
    throw new ExecBaseError(`Error executing "${options.command}" command`, {
      errorMessage: errorMessage,
      stderr: stderr,
      command: options.command,
      args: options.args,
      cwd: cwd,
    });
  }

  logger.debug("Command executed", {
    command: options.command,
    args: options.args,
    cwd: cwd,
    stdout: result.stdout,
    stderr: result.stderr,
  });

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
