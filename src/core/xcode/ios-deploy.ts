import { spawn } from "node:child_process";

import { exec } from "../exec";
import { tempFilePath } from "../files";
import type { Logger } from "../logger/types";
import type { TaskTerminal } from "../tasks/types";

/**
 * Install and launch app on device using ios-deploy
 * This is used for older devices (iOS < 17) that don't support devicectl
 *
 * ios-deploy only works with the legacy UDID format
 */

/**
 * Helper to execute ios-deploy and ignore non-zero exit codes from safequit
 * ios-deploy's safequit often returns non-zero even when the app launches successfully
 * However, we should NOT ignore real errors like command not found, device not found,
 * or signal-based terminations (user pressed Ctrl+C)
 */
async function executeIgnoringExitCode(
  terminal: TaskTerminal,
  command: string,
  args: string[],
  logger: Logger,
): Promise<void> {
  try {
    await terminal.execute({ command, args });
  } catch (error) {
    const execError = error as { exitCode?: number; errorCode?: number | null; stderr?: string; message?: string };
    const exitCode = execError.exitCode ?? execError.errorCode;

    // Exit code 127 = command not found
    if (exitCode === 127) {
      throw error;
    }

    // Exit code null = process killed by signal (SIGTERM/SIGKILL from Ctrl+C)
    // Exit code 130 = SIGINT (Ctrl+C)
    // Exit code 143 = SIGTERM
    if (exitCode === null || exitCode === 130 || exitCode === 143) {
      throw error;
    }

    const stderr = execError.stderr?.toLowerCase() ?? "";
    if (stderr.includes("device not found") || stderr.includes("no device found")) {
      throw error;
    }

    logger.debug("ios-deploy exited with non-zero code (likely safequit), ignoring", { error });
  }
}

/**
 * Stream log file contents to terminal using tail -f
 * This provides real-time console log streaming from ios-deploy output files
 * LLDB output (including app console logs) goes to stderr
 *
 * Returns a cleanup function that terminates the tail process when called.
 */
async function streamLogFile(terminal: TaskTerminal, logFilePath: string, logger: Logger): Promise<() => void> {
  const tailProcess = spawn("tail", ["-f", logFilePath], {
    stdio: ["ignore", "pipe", "pipe"],
  });

  tailProcess.stdout?.on("data", (data: Buffer) => {
    terminal.write(data.toString());
  });

  tailProcess.stderr?.on("data", (data: Buffer) => {
    terminal.write(data.toString(), { color: "yellow" });
  });

  tailProcess.on("error", (error) => {
    logger.debug("Failed to stream log file", { error, logFilePath });
  });

  return () => {
    try {
      tailProcess.kill("SIGTERM");
    } catch {
      // Process already terminated
    }
  };
}

/**
 * Install and launch app on device using ios-deploy (single command)
 * --debug launches the app with LLDB debugger attached
 * User can press Ctrl+C to stop the debugging session when done
 */
export async function installAndLaunchApp(
  terminal: TaskTerminal,
  options: {
    storagePath: string;
    deviceId: string;
    appPath: string;
    bundleId: string;
    launchArgs?: string[];
    launchEnv?: Record<string, string>;
    logger: Logger;
  },
): Promise<void> {
  const logger = options.logger;
  logger.debug("Installing and launching app with ios-deploy", {
    deviceId: options.deviceId,
    appPath: options.appPath,
    bundleId: options.bundleId,
  });

  await using stdoutPath = await tempFilePath(options.storagePath, { prefix: "ios-deploy-stdout" });
  await using stderrPath = await tempFilePath(options.storagePath, { prefix: "ios-deploy-stderr" });

  // Install and launch the app with output file redirection
  // --debug launches the app in lldb after installation
  // --output and --error_output redirect ios-deploy output to files
  // Note: LLDB output (including app console logs) goes to stderr
  const args = [
    "--id",
    options.deviceId,
    "--bundle",
    options.appPath,
    "--debug",
    "--unbuffered",
    "--output",
    stdoutPath.path,
    "--error_output",
    stderrPath.path,
  ];

  if (options.launchArgs && options.launchArgs.length > 0) {
    args.push("--args", ...options.launchArgs);
  }

  if (options.launchEnv) {
    for (const [key, value] of Object.entries(options.launchEnv)) {
      args.push("--env", `${key}=${value}`);
    }
  }

  // Start streaming stderr in background; LLDB writes app console logs there.
  const stopStreaming = await streamLogFile(terminal, stderrPath.path, logger);

  try {
    await executeIgnoringExitCode(terminal, "ios-deploy", args, logger);
  } finally {
    stopStreaming();
  }
}

/**
 * Check if ios-deploy is installed
 */
export async function isIosDeployInstalled(options: { cwd: string; logger: Logger }): Promise<boolean> {
  try {
    await exec({
      command: "ios-deploy",
      args: ["--version"],
      cwd: options.cwd,
      logger: options.logger,
    });
    return true;
  } catch {
    return false;
  }
}
