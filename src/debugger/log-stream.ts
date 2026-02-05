import { type ChildProcess, spawn } from "node:child_process";
import * as vscode from "vscode";
import type { ExtensionContext, LastLaunchedAppContext } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { commonLogger } from "../common/logger";

/**
 * Builds the default predicate for filtering logs from the unified logging system.
 *
 * The predicate captures:
 * 1. os_log - logs with subsystem matching the app's bundle identifier
 * 2. NSLog - logs sent via Foundation framework from the app's process
 * 3. print() - captured via stdout in onOutputLine callback
 *
 * Available predicate fields (from `man log`):
 * - eventType, eventMessage, messageType
 * - process, processImagePath
 * - sender, senderImagePath (library/framework that originated the log)
 * - subsystem, category
 */
function buildDefaultPredicate(bundleIdentifier: string, processName: string): string {
  // os_log: Uses the bundle identifier as the subsystem
  const osLogPredicate = `subsystem BEGINSWITH "${bundleIdentifier}"`;

  // NSLog: Logs via Foundation framework, no subsystem set
  // We filter by process name AND sender to avoid capturing other Foundation logs
  const nsLogPredicate = `(process == "${processName}" AND sender == "Foundation")`;

  return `${osLogPredicate} OR ${nsLogPredicate}`;
}

/**
 * Manages app output streaming for debug sessions.
 *
 * This class handles two types of output:
 * 1. os_log - captured via `log stream` command with predicate filtering
 * 2. NSLog - captured via `log stream` command with predicate filtering
 * 3. print() - forwarded from the task terminal via onOutputLine callback
 *
 * Both types of output are displayed in the "SweetPad: App Logs" output channel.
 */
export class LogStreamManager {
  private _context: ExtensionContext;
  private outputChannel: vscode.OutputChannel | undefined;
  private logStreamProcess: ChildProcess | undefined;
  private sessionDisposable: vscode.Disposable | undefined;

  constructor(context: ExtensionContext) {
    this._context = context;
  }

  /**
   * Get or create the output channel for app logs.
   * This is exposed so the build manager can forward stdout/stderr to it.
   */
  getOutputChannel(): vscode.OutputChannel {
    if (!this.outputChannel) {
      this.outputChannel = vscode.window.createOutputChannel("SweetPad: App Logs");
    }
    return this.outputChannel;
  }

  /**
   * Prepare the output channel for a new launch session.
   * Called by the build manager before launching the app.
   */
  prepareForLaunch(bundleIdentifier: string): void {
    const isEnabled = getWorkspaceConfig("build.logStreamEnabled") ?? true;
    if (!isEnabled) {
      return;
    }

    const channel = this.getOutputChannel();
    channel.clear();
    channel.show(true);
    channel.appendLine(`[SweetPad] Launching app: ${bundleIdentifier}`);
    channel.appendLine("[SweetPad] print() output will appear below.");
    channel.appendLine("");
  }

  /**
   * Append a line of output from the app's stdout/stderr.
   * Called by the build manager's onOutputLine callback.
   *
   * This captures print() and debugPrint() output which goes to stdout,
   * not through the unified logging system.
   */
  appendOutput(line: string, type: "stdout" | "stderr"): void {
    const isEnabled = getWorkspaceConfig("build.logStreamEnabled") ?? true;
    if (!isEnabled) {
      return;
    }

    const channel = this.getOutputChannel();
    if (type === "stderr") {
      channel.appendLine(`[stderr] ${line}`);
    } else {
      channel.appendLine(line);
    }
  }

  /**
   * Start log streaming for the given launch context.
   * This captures logs from the unified logging system (os_log/Logger/NSLog).
   */
  async startLogStream(launchContext: LastLaunchedAppContext): Promise<void> {
    // Stop any existing log stream before starting a new one
    this.stopLogStream();

    const isEnabled = getWorkspaceConfig("build.logStreamEnabled") ?? true;
    if (!isEnabled) {
      commonLogger.debug("Log stream is disabled via configuration");
      return;
    }

    const bundleIdentifier = launchContext.bundleIdentifier;
    if (!bundleIdentifier) {
      commonLogger.warn("Cannot start log stream: bundle identifier not available");
      return;
    }

    // Extract process name from bundle ID (last component)
    // e.g., "com.example.MyApp" -> "MyApp"
    const processName = bundleIdentifier.split(".").pop() ?? bundleIdentifier;

    // Build the predicate - use custom if configured, otherwise use default
    const customPredicate = getWorkspaceConfig("build.logStreamPredicate");
    const predicate = customPredicate
      ? customPredicate.replace(/\$\{bundleId\}/g, bundleIdentifier).replace(/\$\{processName\}/g, processName)
      : buildDefaultPredicate(bundleIdentifier, processName);

    try {
      this.spawnLogStreamProcess(launchContext, predicate);
    } catch (error) {
      commonLogger.error("Failed to start log stream", { error });
    }

    // Add separator between stdout and unified logging output
    const channel = this.getOutputChannel();
    channel.appendLine("");
    channel.appendLine("─".repeat(60));
    channel.appendLine("[SweetPad] os_log / Logger / NSLog output:");
    channel.appendLine(`[SweetPad] Predicate: ${predicate}`);
    channel.appendLine("");

    // Register handler to stop log stream when debug session ends
    this.sessionDisposable = vscode.debug.onDidTerminateDebugSession(() => {
      this.stopLogStream();
    });

    // Show the output channel
    channel.show();
  }

  /**
   * Spawn the log stream process based on the launch context type.
   */
  private spawnLogStreamProcess(launchContext: LastLaunchedAppContext, predicate: string): void {
    let command: string;
    let args: string[];

    switch (launchContext.type) {
      case "simulator": {
        // For simulators, use xcrun simctl spawn to run log stream inside the simulator
        command = "xcrun";
        args = [
          "simctl",
          "spawn",
          launchContext.simulatorUdid,
          "log",
          "stream",
          "--predicate",
          predicate,
          "--level",
          "debug",
          "--style",
          "compact",
        ];
        break;
      }
      case "macos": {
        // For macOS, run log stream directly
        command = "log";
        args = ["stream", "--predicate", predicate, "--level", "debug", "--style", "compact"];
        break;
      }
      case "device": {
        // For physical devices, log streaming via devicectl is not yet supported
        // TODO: investigate devicectl device process log stream support
        const channel = this.getOutputChannel();
        channel.appendLine("[SweetPad] os_log streaming for physical devices is not yet supported.");
        channel.appendLine("[SweetPad] Use Console.app or Xcode to view device logs.");
        return;
      }
    }

    commonLogger.debug("Starting log stream", { command, args });

    const subprocess = spawn(command, args, {
      stdio: ["ignore", "pipe", "pipe"],
    });

    this.logStreamProcess = subprocess;

    const channel = this.getOutputChannel();

    subprocess.stdout?.on("data", (data: Buffer) => {
      channel.append(data.toString());
    });

    subprocess.stderr?.on("data", (data: Buffer) => {
      channel.append(`[log stream stderr] ${data.toString()}`);
    });

    subprocess.on("exit", (code: number | null, signal: string | null) => {
      if (code !== null && code !== 0) {
        channel.appendLine(`\n[SweetPad] Log stream exited with code ${code}`);
      } else if (signal) {
        channel.appendLine(`\n[SweetPad] Log stream terminated by signal ${signal}`);
      }
      this.logStreamProcess = undefined;
    });

    subprocess.on("error", (error: Error) => {
      commonLogger.error("Log stream process error", { error });
      channel.appendLine(`\n[SweetPad] Log stream error: ${error.message}`);
      this.logStreamProcess = undefined;
    });
  }

  /**
   * Stop the log stream process and clean up resources.
   */
  stopLogStream(): void {
    if (this.logStreamProcess) {
      commonLogger.debug("Stopping log stream process");
      this.logStreamProcess.kill("SIGTERM");
      this.logStreamProcess = undefined;
    }

    if (this.sessionDisposable) {
      this.sessionDisposable.dispose();
      this.sessionDisposable = undefined;
    }
  }

  /**
   * Dispose of all resources.
   */
  dispose(): void {
    this.stopLogStream();
    if (this.outputChannel) {
      this.outputChannel.dispose();
      this.outputChannel = undefined;
    }
  }
}

/**
 * Singleton instance of LogStreamManager.
 * This allows the build manager to access the same output channel as the debug provider.
 */
let logStreamManagerInstance: LogStreamManager | undefined;

/**
 * Get or create the singleton LogStreamManager instance.
 */
export function getLogStreamManager(context: ExtensionContext): LogStreamManager {
  if (!logStreamManagerInstance) {
    logStreamManagerInstance = new LogStreamManager(context);
  }
  return logStreamManagerInstance;
}

/**
 * Dispose the singleton LogStreamManager instance.
 * Called when the extension is deactivated.
 */
export function disposeLogStreamManager(): void {
  if (logStreamManagerInstance) {
    logStreamManagerInstance.dispose();
    logStreamManagerInstance = undefined;
  }
}
