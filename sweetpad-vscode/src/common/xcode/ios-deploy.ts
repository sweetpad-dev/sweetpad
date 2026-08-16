import type * as vscode from "vscode";

import { exec } from "../exec";
import { tempFilePath } from "../files";
import { commonLogger } from "../logger";
import type { ProcessGroup, TaskTerminal } from "../tasks/types";

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
  vscodeContext: vscode.ExtensionContext,
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
  await using stdoutPath = await tempFilePath(vscodeContext, { prefix: "ios-deploy-stdout" });
  await using stderrPath = await tempFilePath(vscodeContext, { prefix: "ios-deploy-stderr" });

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
 * Coordinates ios-deploy reports once its debug phase is up, parsed out of the lines it
 * prints on the way there.
 */
export type IosDeployDebugserver = {
  port: number;
  deviceAppPath: string;
  symbolsPath?: string;
};

// Matched against the merged pty stream, where every line ends in "\r\n". "." excludes
// carriage returns, so the trailing "\r" has to be consumed explicitly.
//   "debugserver port: 12345"
//   "App path: /private/var/containers/Bundle/Application/C82BF61B-.../MyApp.app"
//   "Symbol Path: /Users/me/Library/Developer/Xcode/iOS DeviceSupport/iPad5,1 15.6.1 (19G82)/Symbols"
const DEBUGSERVER_PORT_RE = /^debugserver port:[ \t]*(\d+)[ \t]*\r?$/m;
const DEVICE_APP_PATH_RE = /^App path:[ \t]*(.+?)[ \t]*\r?$/m;
const SYMBOLS_PATH_RE = /^Symbol Path:[ \t]*(.+?)[ \t]*\r?$/m;

/**
 * Pull the debug-phase coordinates out of whatever ios-deploy has printed so far. Fields are
 * absent until their line shows up, so this is safe to call on a partial stream.
 */
export function parseDebugserverOutput(output: string): Partial<IosDeployDebugserver> {
  const port = DEBUGSERVER_PORT_RE.exec(output)?.[1];
  return {
    port: port === undefined ? undefined : Number.parseInt(port, 10),
    deviceAppPath: DEVICE_APP_PATH_RE.exec(output)?.[1],
    symbolsPath: SYMBOLS_PATH_RE.exec(output)?.[1],
  };
}

/**
 * A running ios-deploy holding a debugserver open. "wait" resolves when it exits, which is
 * what keeps the launch task alive for the length of the debug session.
 */
export type DebugserverSession = {
  debugserver: IosDeployDebugserver;
  wait(): Promise<void>;
};

/** Tail of ios-deploy's output, for error messages when it dies before the debug phase. */
function outputTail(output: string, lines: number): string {
  const trimmed = output.replace(/\s+$/, "").split("\n");
  return trimmed.slice(-lines).join("\n").trim();
}

function startDebugserver(
  group: ProcessGroup,
  options: {
    deviceId: string;
    appPath: string;
    timeoutMs: number;
  },
): Promise<DebugserverSession> {
  // --nolldb puts ios-deploy in its debug phase but leaves LLDB to us: it installs the app,
  // mounts the developer disk image, starts debugserver and prints the port. The app is not
  // started — LLDB launches it through debugserver, so breakpoints in startup code are live
  // before any app code runs.
  const handle = group.spawn({
    command: "ios-deploy",
    args: ["--id", options.deviceId, "--bundle", options.appPath, "--nolldb", "--unbuffered"],
    pty: true,
    main: true,
  });

  return new Promise<DebugserverSession>((resolve, reject) => {
    let output = "";
    let settled = false;

    const timeout = setTimeout(() => {
      if (settled) {
        return;
      }
      settled = true;
      handle.kill();
      const error = new Error(
        `Timed out waiting for ios-deploy to start a debugserver. Last output:\n${outputTail(output, 10)}`,
      );
      // Reject only once the kill has landed. The process group refuses a second "main"
      // child while the first is alive, so rejecting straight away would make the retry
      // fail on spawn — and replace this message with that failure.
      handle.exit.then(
        () => reject(error),
        () => reject(error),
      );
    }, options.timeoutMs);

    handle.onData((chunk) => {
      group.terminal.write(chunk);
      if (settled) {
        return;
      }

      output += chunk;
      const parsed = parseDebugserverOutput(output);
      if (parsed.port === undefined || !parsed.deviceAppPath) {
        return;
      }

      settled = true;
      clearTimeout(timeout);
      commonLogger.debug("ios-deploy debugserver ready", { ...parsed });
      resolve({
        debugserver: { port: parsed.port, deviceAppPath: parsed.deviceAppPath, symbolsPath: parsed.symbolsPath },
        wait: async () => {
          await handle.exit;
        },
      });
    });

    handle.exit.then((exit) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timeout);
      reject(
        new Error(
          `ios-deploy exited with code ${exit.code} before starting a debugserver. Last output:\n${outputTail(output, 10)}`,
        ),
      );
    });
  });
}

/**
 * Install the app and leave a debugserver listening for LLDB, for devices without CoreDevice
 * (iOS 16 and below, where "devicectl" is unavailable).
 *
 * The first connection to such a device fails intermittently inside ios-deploy's own retry
 * logic, so a failure to reach the debug phase is retried once before giving up.
 */
export async function launchDebugserver(
  group: ProcessGroup,
  options: {
    deviceId: string;
    appPath: string;
    timeoutMs?: number;
    attempts?: number;
  },
): Promise<DebugserverSession> {
  const attempts = options.attempts ?? 2;
  const timeoutMs = options.timeoutMs ?? 120_000;

  let lastError: unknown;
  for (let attempt = 1; attempt <= attempts; attempt++) {
    try {
      return await startDebugserver(group, { deviceId: options.deviceId, appPath: options.appPath, timeoutMs });
    } catch (error) {
      lastError = error;
      commonLogger.debug("ios-deploy failed to start a debugserver", { attempt, error });
      if (attempt < attempts) {
        group.terminal.write(`[sweetpad] ios-deploy did not reach its debug phase, retrying`, {
          newLine: true,
          color: "yellow",
        });
      }
    }
  }
  throw lastError;
}

/**
 * Check if ios-deploy is installed
 */
export async function isIosDeployInstalled(): Promise<boolean> {
  try {
    await exec({
      command: "ios-deploy",
      args: ["--version"],
      cwd: null,
    });
    return true;
  } catch {
    return false;
  }
}
