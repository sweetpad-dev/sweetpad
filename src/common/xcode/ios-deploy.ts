import type { ExtensionContext } from "../commands";
import { exec } from "../exec";
import { commonLogger } from "../logger";
import type { TaskTerminal } from "../tasks";
import { tempFilePath } from "../files";

/**
 * Install and launch app on device using ios-deploy
 * This is used for older devices (iOS < 17) that don't support devicectl
 *
 * ios-deploy only works with the legacy UDID format
 */

/**
 * Helper to execute ios-deploy and ignore non-zero exit codes from safequit
 * ios-deploy's safequit often returns non-zero even when the app launches successfully
 * However, we should NOT ignore real errors like command not found or device not found
 */
async function executeIgnoringExitCode(terminal: TaskTerminal, command: string, args: string[]): Promise<void> {
  try {
    await terminal.execute({ command, args });
  } catch (error) {
    // Check if this is a real error we should re-throw
    const execError = error as { exitCode?: number; stderr?: string; message?: string };

    // Exit code 127 = command not found
    if (execError.exitCode === 127) {
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
 */
async function streamLogFile(terminal: TaskTerminal, logFilePath: string): Promise<void> {
  const tailCommand = "tail";
  const tailArgs = ["-f", logFilePath];

  try {
    await terminal.execute({ command: tailCommand, args: tailArgs });
  } catch (error) {
    // If tail fails (e.g., file doesn't exist yet), just log and continue
    commonLogger.debug("Failed to stream log file", { error, logFilePath });
  }
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
    "--id", options.deviceId,
    "--bundle", options.appPath,
    "--debug",
    "--unbuffered",
    "--output", stdoutPath.path,
    "--error_output", stderrPath.path,
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

  // Start streaming the stderr file in background
  // Note: LLDB output (including app console logs) goes to stderr, not stdout
  void streamLogFile(terminal, stderrPath.path);

  await executeIgnoringExitCode(terminal, "ios-deploy", args);
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
