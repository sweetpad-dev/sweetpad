import type { ExtensionContext } from "../commands";
import { exec } from "../exec";
import { tempFilePath } from "../files";
import { commonLogger } from "../logger";
import type { TaskTerminal } from "../tasks";

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
async function executeIgnoringExitCode(terminal: TaskTerminal, command: string, args: string[]): Promise<void> {
  try {
    await terminal.execute({ command, args });
  } catch (error) {
    // Check if this is a real error we should re-throw
    const execError = error as { exitCode?: number; errorCode?: number | null; stderr?: string; message?: string };
    const exitCode = execError.exitCode ?? execError.errorCode;

    // Exit code 127 = command not found
    if (exitCode === 127) {
      throw error;
    }

    // Exit code null = process killed by signal (SIGTERM/SIGKILL from Ctrl+C)
    // Exit code 130 = SIGINT (Ctrl+C)
    // Exit code 143 = SIGTERM
    // These indicate user-initiated cancellation and must propagate
    if (exitCode === null || exitCode === 130 || exitCode === 143) {
      throw error;
    }

    // Check stderr for device-related errors
    const stderr = execError.stderr?.toLowerCase() ?? "";
    if (stderr.includes("device not found") || stderr.includes("no device found")) {
      throw error;
    }

    // For other non-zero exits (likely safequit), just log and ignore
    commonLogger.debug("ios-deploy exited with non-zero code (likely safequit), ignoring", { error });
  }
}

/**
 * Stream log file contents to terminal using tail -f
 * This provides real-time console log streaming from ios-deploy output files
 * LLDB output (including app console logs) goes to stderr
 *
 * Returns a cleanup function that terminates the tail process when called.
 */
async function streamLogFile(terminal: TaskTerminal, logFilePath: string): Promise<() => void> {
  // We need to start tail -f in a way that we can cancel it when ios-deploy exits.
  // Instead of calling terminal.execute() (which would race with ios-deploy for
  // this.process ownership), we use a separate spawn to avoid the race condition.
  const { spawn } = await import("node:child_process");
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
    commonLogger.debug("Failed to stream log file", { error, logFilePath });
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
  context: ExtensionContext,
  terminal: TaskTerminal,
  options: {
    deviceId: string;
    appPath: string;
    bundleId: string;
    launchArgs?: string[];
    launchEnv?: Record<string, string>;
  },
): Promise<void> {
  commonLogger.debug("Installing and launching app with ios-deploy", {
    deviceId: options.deviceId,
    appPath: options.appPath,
    bundleId: options.bundleId,
  });

  // Create temporary files for capturing console output
  await using stdoutPath = await tempFilePath(context, { prefix: "ios-deploy-stdout" });
  await using stderrPath = await tempFilePath(context, { prefix: "ios-deploy-stderr" });

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

  // Add launch arguments if provided
  if (options.launchArgs && options.launchArgs.length > 0) {
    args.push("--args", ...options.launchArgs);
  }

  // Add environment variables if provided
  if (options.launchEnv) {
    for (const [key, value] of Object.entries(options.launchEnv)) {
      args.push("--env", `${key}=${value}`);
    }
  }

  // Start streaming the stderr file in background and get cleanup function
  // Note: LLDB output (including app console logs) goes to stderr, not stdout
  const stopStreaming = await streamLogFile(terminal, stderrPath.path);

  try {
    await executeIgnoringExitCode(terminal, "ios-deploy", args);
  } finally {
    // Always stop the tail process when ios-deploy exits (success, error, or Ctrl+C)
    stopStreaming();
  }
}

/**
 * Check if ios-deploy is installed
 */
export async function isIosDeployInstalled(): Promise<boolean> {
  try {
    await exec({
      command: "ios-deploy",
      args: ["--version"],
    });
    return true;
  } catch {
    return false;
  }
}
