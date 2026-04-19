import { type ChildProcess, spawn } from "node:child_process";
import * as vscode from "vscode";
import type { ExtensionContext, LastLaunchedAppContext, LastLaunchedAppDeviceContext } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { exec } from "../common/exec";
import { commonLogger } from "../common/logger";
import { buildPymobiledevice3Args, formatCommandLine } from "./device-log-backend";
import { type LogPipe, PassthroughLogPipe, Pymobiledevice3LogPipe } from "./log-pipe";

type LogStreamSpec = {
  command: string;
  args: string[];
  stderrPrefix: string;
  lineProcessor: LogPipe;
};

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

    const channel = this.getOutputChannel();
    channel.appendLine("");
    channel.appendLine("─".repeat(60));
    if (launchContext.type === "device") {
      // Device backends don't use a `log stream` predicate. Each backend prints its own header below.
      channel.appendLine("[SweetPad] Device log streaming:");
    } else {
      channel.appendLine("[SweetPad] os_log / Logger / NSLog output:");
      channel.appendLine(`[SweetPad] Predicate: ${predicate}`);
    }
    channel.appendLine("");

    try {
      await this.spawnLogStreamProcess(launchContext, predicate);
    } catch (error) {
      commonLogger.error("Failed to start log stream", { error });
    }

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
  private async spawnLogStreamProcess(launchContext: LastLaunchedAppContext, predicate: string): Promise<void> {
    const channel = this.getOutputChannel();
    const spec = await this.resolveLogStreamSpec(launchContext, predicate, channel);
    if (!spec) {
      return;
    }

    commonLogger.debug("Starting log stream", { command: spec.command, args: spec.args });
    const subprocess = spawn(spec.command, spec.args, { stdio: ["ignore", "pipe", "pipe"] });
    this.logStreamProcess = subprocess;
    this.attachSubprocessHandlers(subprocess, channel, spec.stderrPrefix, spec.lineProcessor);
  }

  private async resolveLogStreamSpec(
    launchContext: LastLaunchedAppContext,
    predicate: string,
    channel: vscode.OutputChannel,
  ): Promise<LogStreamSpec | null> {
    switch (launchContext.type) {
      case "simulator":
        return this.buildSimulatorSpec(launchContext.simulatorUdid, predicate, channel);
      case "macos":
        return this.buildMacOSSpec(predicate, channel);
      case "device":
        return this.resolveDeviceSpec(launchContext, channel);
    }
  }

  private buildSimulatorSpec(simulatorUdid: string, predicate: string, channel: vscode.OutputChannel): LogStreamSpec {
    return {
      command: "xcrun",
      args: [
        "simctl",
        "spawn",
        simulatorUdid,
        "log",
        "stream",
        "--predicate",
        predicate,
        "--level",
        "debug",
        "--style",
        "compact",
      ],
      stderrPrefix: "[log stream stderr]",
      lineProcessor: new PassthroughLogPipe((text) => channel.append(text)),
    };
  }

  private buildMacOSSpec(predicate: string, channel: vscode.OutputChannel): LogStreamSpec {
    return {
      command: "log",
      args: ["stream", "--predicate", predicate, "--level", "debug", "--style", "compact"],
      stderrPrefix: "[log stream stderr]",
      lineProcessor: new PassthroughLogPipe((text) => channel.append(text)),
    };
  }

  private async resolveDeviceSpec(
    launchContext: LastLaunchedAppDeviceContext,
    channel: vscode.OutputChannel,
  ): Promise<LogStreamSpec | null> {
    const backend = launchContext.logBackend ?? "off";
    switch (backend) {
      case "osActivityDtMode":
        channel.appendLine(
          "[SweetPad] os_log/Logger output is being mirrored to stderr via OS_ACTIVITY_DT_MODE=enable.",
        );
        channel.appendLine("[SweetPad] Messages appear in the stdout/stderr stream above.");
        channel.appendLine(
          "[SweetPad] Caveat: mirrors main-executable logs only; dynamically-loaded framework logs may be missing.",
        );
        return null;
      case "pymobiledevice3":
        return await this.buildPymobiledevice3Spec(launchContext, channel);
      default:
        channel.appendLine("[SweetPad] Device os_log streaming is disabled (build.deviceLogStreamBackend=off).");
        channel.appendLine("[SweetPad] Use Console.app or Xcode to view device logs.");
        return null;
    }
  }

  private async buildPymobiledevice3Spec(
    launchContext: LastLaunchedAppDeviceContext,
    channel: vscode.OutputChannel,
  ): Promise<LogStreamSpec | null> {
    const binaryPath = getWorkspaceConfig("build.pymobiledevice3Path") ?? "pymobiledevice3";
    if (!(await isPymobiledevice3Available(binaryPath))) {
      channel.appendLine(`[SweetPad] '${binaryPath}' not found on PATH.`);
      channel.appendLine("[SweetPad] Install with: pip install pymobiledevice3");
      channel.appendLine("[SweetPad] Or set 'sweetpad.build.pymobiledevice3Path' to an absolute path.");
      return null;
    }
    const rawExtraArgs = getWorkspaceConfig("build.pymobiledevice3ExtraArgs") ?? [];
    const result = buildPymobiledevice3Args({
      rawExtraArgs,
      processName: launchContext.executableName,
    });
    if (result.kind === "missingProcessName") {
      channel.appendLine(
        "[SweetPad] Could not determine the process name for log filtering (EXECUTABLE_NAME missing).",
      );
      channel.appendLine(
        "[SweetPad] Set 'sweetpad.build.pymobiledevice3ExtraArgs' to include '--process-name <name>'.",
      );
      return null;
    }
    channel.appendLine(`[SweetPad] Extra args: ${JSON.stringify(rawExtraArgs)}`);
    channel.appendLine(`[SweetPad] Command: ${formatCommandLine(binaryPath, result.args)}`);
    channel.appendLine("[SweetPad] On iOS 17+, requires a running tunnel: sudo pymobiledevice3 remote tunneld");

    // We drop "--match" / "--regex" at the CLI and instead parse each line into
    // a structured entry locally, then filter by image name. This fixes the two
    // gaps with server-side matching (see PR #231): framework-emitted lines
    // whose subsystem happens to contain the bundle id, and app-emitted lines
    // whose custom `Logger(subsystem:)` doesn't.
    //
    // When the override replaces "--process-name", prefer that for the filter
    // — the user knows better than us what image names the app produces.
    const filterExecutable = extractProcessNameOverride(rawExtraArgs) ?? launchContext.executableName;
    const debugDylibOnly = launchContext.enableDebugDylib ?? true;
    const subsystemDenyList = getWorkspaceConfig("build.pymobiledevice3SubsystemDenyList") ?? ["com.apple.*"];
    const subsystemAllowList = getWorkspaceConfig("build.pymobiledevice3SubsystemAllowList") ?? [];

    const output = (text: string) => channel.appendLine(text);
    let lineProcessor: LogPipe;
    if (filterExecutable) {
      lineProcessor = new Pymobiledevice3LogPipe(output, {
        executableName: filterExecutable,
        debugDylibOnly,
        subsystemDenyList,
        subsystemAllowList,
      });
      if (debugDylibOnly) {
        channel.appendLine(`[SweetPad] Image filter: only '${filterExecutable}.debug.dylib'.`);
      } else {
        channel.appendLine(`[SweetPad] Image filter: '${filterExecutable}' + '${filterExecutable}.debug.dylib'.`);
      }
      if (subsystemDenyList.length > 0) {
        channel.appendLine(`[SweetPad] Subsystem deny-list: ${JSON.stringify(subsystemDenyList)}`);
      }
      if (subsystemAllowList.length > 0) {
        channel.appendLine(`[SweetPad] Subsystem allow-list: ${JSON.stringify(subsystemAllowList)}`);
      }
    } else {
      lineProcessor = new PassthroughLogPipe(output);
      channel.appendLine("[SweetPad] Local filter disabled (no executable name available).");
    }

    return {
      command: binaryPath,
      args: result.args,
      stderrPrefix: "[pymobiledevice3]",
      lineProcessor,
    };
  }

  private attachSubprocessHandlers(
    subprocess: ChildProcess,
    channel: vscode.OutputChannel,
    stderrPrefix: string,
    lineProcessor: LogPipe,
  ): void {
    subprocess.stdout?.on("data", (data: Buffer) => lineProcessor.push(data.toString()));
    subprocess.stdout?.on("end", () => lineProcessor.flush());
    subprocess.stderr?.on("data", (data: Buffer) => {
      channel.append(`${stderrPrefix} ${data.toString()}`);
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
 * When the user puts "--process-name NAME" / "-p NAME" into the extra args, we
 * honor it for the local image-name filter too. A null value (used to suppress
 * the default) yields undefined so we fall back to `launchContext.executableName`.
 */
function extractProcessNameOverride(rawExtraArgs: (string | null)[]): string | undefined {
  for (let i = 0; i < rawExtraArgs.length; i++) {
    const arg = rawExtraArgs[i];
    if (arg === "--process-name" || arg === "-p") {
      const value = rawExtraArgs[i + 1];
      if (typeof value === "string") {
        return value;
      }
      return undefined;
    }
  }
  return undefined;
}

async function isPymobiledevice3Available(binaryPath: string): Promise<boolean> {
  try {
    await exec({ command: binaryPath, args: ["version"] });
    return true;
  } catch {
    return false;
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
