import { type ChildProcess, spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { getWorkspaceConfig } from "../common/config";
import { exec } from "../common/exec";
import { commonLogger } from "../common/logger";
import type { TaskTerminal } from "../common/tasks";
import { type LogFilter, PassthroughLogFilter, Pymobiledevice3LogFilter } from "./filters";

export type LoggingManagerOptions =
  | { type: "simulator"; bundleIdentifier: string; simulatorUdid: string }
  | { type: "macos"; bundleIdentifier: string }
  | { type: "device"; bundleIdentifier: string; executableName?: string };

/**
 * Manages a single os_log/Logger stream from the unified logging system.
 *
 * Create a new instance for each app launch. The caller is responsible for
 * calling stop() when done (typically in a try/finally block around the
 * app execution).
 */
export class LoggingManager {
  private logStreamProcess: ChildProcess | undefined;
  private terminal: TaskTerminal;
  private options: LoggingManagerOptions;

  private writeLogLine = (text: string) => {
    this.terminal.write(`[os_log] ${text}`, { color: "cyan", newLine: true });
  };

  constructor(terminal: TaskTerminal, options: LoggingManagerOptions) {
    this.terminal = terminal;
    this.options = options;
  }

  /**
   * Start log streaming.
   * Captures os_log/Logger output from the unified logging system.
   */
  async start(): Promise<void> {
    const isEnabled = getWorkspaceConfig("build.logStreamEnabled") ?? true;
    if (!isEnabled) {
      commonLogger.debug("Log stream is disabled via configuration");
      return;
    }

    const { bundleIdentifier } = this.options;

    // Extract process name from bundle ID (last component)
    // e.g., "com.example.MyApp" -> "MyApp"
    const processName = bundleIdentifier.split(".").pop() ?? bundleIdentifier;

    // Build the predicate - use custom if configured, otherwise use default
    const customPredicate = getWorkspaceConfig("build.logStreamPredicate");
    const predicate = customPredicate
      ? customPredicate.replace(/\$\{bundleId\}/g, bundleIdentifier).replace(/\$\{processName\}/g, processName)
      : `subsystem BEGINSWITH "${bundleIdentifier}"`;

    commonLogger.debug("Log stream predicate", { predicate });

    try {
      switch (this.options.type) {
        case "simulator":
          this.startSimulatorStream(this.options.simulatorUdid, predicate);
          break;
        case "macos":
          this.startMacOSStream(predicate);
          break;
        case "device":
          await this.startDeviceStream(this.options.executableName);
          break;
      }
    } catch (error) {
      commonLogger.error("Failed to start log stream", { error });
    }
  }

  private startSimulatorStream(simulatorUdid: string, predicate: string): void {
    this.spawnProcess(
      "xcrun",
      [
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
      "[log stream stderr]",
      new PassthroughLogFilter(),
    );
  }

  private startMacOSStream(predicate: string): void {
    this.spawnProcess(
      "log",
      ["stream", "--predicate", predicate, "--level", "debug", "--style", "compact"],
      "[log stream stderr]",
      new PassthroughLogFilter(),
    );
  }

  private async startDeviceStream(executableName: string | undefined): Promise<void> {
    const backend = resolveDeviceLogBackend();
    switch (backend) {
      case "osActivityDtMode":
        commonLogger.debug("os_log mirrored to stderr via OS_ACTIVITY_DT_MODE=enable");
        return;
      case "pymobiledevice3":
        await this.startPymobiledevice3Stream(executableName);
        return;
      default:
        commonLogger.debug("Device os_log streaming is disabled (build.deviceLoggingManagerBackend=off)");
        return;
    }
  }

  private async startPymobiledevice3Stream(executableName: string | undefined): Promise<void> {
    const binaryPath = getWorkspaceConfig("build.pymobiledevice3Path") ?? "pymobiledevice3";
    if (!(await isPymobiledevice3Available(binaryPath))) {
      this.terminal.write(`[SweetPad] '${binaryPath}' not found. Install with: pip install pymobiledevice3`, {
        newLine: true,
      });
      return;
    }
    const rawExtraArgs = getWorkspaceConfig("build.pymobiledevice3ExtraArgs") ?? [];
    const result = buildPymobiledevice3Args({
      rawExtraArgs,
      processName: executableName,
    });
    if (result.kind === "missingProcessName") {
      this.terminal.write("[SweetPad] Could not determine process name for log filtering (EXECUTABLE_NAME missing).", {
        newLine: true,
      });
      return;
    }
    commonLogger.debug("pymobiledevice3 log stream", {
      extraArgs: rawExtraArgs,
      command: [binaryPath, ...result.args].map(shellQuote).join(" "),
    });

    // We drop "--match" / "--regex" at the CLI and instead parse each line into
    // a structured entry locally, then filter by image name. This fixes the two
    // gaps with server-side matching (see PR #231): framework-emitted lines
    // whose subsystem happens to contain the bundle id, and app-emitted lines
    // whose custom `Logger(subsystem:)` doesn't.
    //
    // When the override replaces "--process-name", prefer that for the filter
    // — the user knows better than us what image names the app produces.
    const filterExecutable = extractProcessNameOverride(rawExtraArgs) ?? executableName;
    const debugDylibOnly = getWorkspaceConfig("build.pymobiledevice3DebugDylibOnly") ?? true;
    const subsystemDenyList = getWorkspaceConfig("build.pymobiledevice3SubsystemDenyList") ?? ["com.apple.*"];
    const subsystemAllowList = getWorkspaceConfig("build.pymobiledevice3SubsystemAllowList") ?? [];

    let filter: LogFilter;
    if (filterExecutable) {
      filter = new Pymobiledevice3LogFilter({
        executableName: filterExecutable,
        debugDylibOnly,
        subsystemDenyList,
        subsystemAllowList,
      });
    } else {
      filter = new PassthroughLogFilter();
    }

    this.spawnProcess(binaryPath, result.args, "[pymobiledevice3]", filter);
  }

  private spawnProcess(command: string, args: string[], stderrPrefix: string, filter: LogFilter): void {
    commonLogger.debug("Starting log stream", { command, args });
    const subprocess = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"] });
    this.logStreamProcess = subprocess;

    const rl = createInterface({ input: subprocess.stdout });
    rl.on("line", (line) => {
      const result = filter.processLine(line);
      if (result !== null) {
        this.writeLogLine(result);
      }
    });
    subprocess.stderr?.on("data", (data: Buffer) => {
      this.terminal.write(`${stderrPrefix} ${data.toString()}`, { newLine: true });
    });
    subprocess.on("exit", (code: number | null, signal: string | null) => {
      if (code !== null && code !== 0) {
        this.terminal.write(`[SweetPad] Log stream exited with code ${code}`, { newLine: true });
      } else if (signal) {
        this.terminal.write(`[SweetPad] Log stream terminated by signal ${signal}`, { newLine: true });
      }
      this.logStreamProcess = undefined;
    });
    subprocess.on("error", (error: Error) => {
      commonLogger.error("Log stream process error", { error });
      this.terminal.write(`[SweetPad] Log stream error: ${error.message}`, { newLine: true });
      this.logStreamProcess = undefined;
    });
  }

  /**
   * Stop the log stream process and clean up resources.
   */
  stop(): void {
    if (this.logStreamProcess) {
      commonLogger.debug("Stopping log stream process");
      this.logStreamProcess.kill("SIGTERM");
      this.logStreamProcess = undefined;
    }
  }
}

type Pymobiledevice3ArgsResult =
  | { kind: "ok"; args: string[]; hasProcessNameOverride: boolean }
  | { kind: "missingProcessName" };

/**
 * Build the argv for `pymobiledevice3 syslog live`, merging SweetPad's defaults
 * with user-supplied extras.
 *
 * Rules:
 * - "--process-name" / "-p" in extras fully replaces SweetPad's default.
 * - A "null" value after the flag suppresses SweetPad's default without
 *   adding a replacement.
 * - Any other args pass through in order.
 * - If the process name is missing AND no override was provided, returns
 *   `missingProcessName`.
 */
export function buildPymobiledevice3Args(input: {
  rawExtraArgs: (string | null)[];
  processName: string | undefined;
}): Pymobiledevice3ArgsResult {
  const { rawExtraArgs, processName } = input;

  let hasProcessNameOverride = false;
  const cleanedExtra: string[] = [];

  for (let i = 0; i < rawExtraArgs.length; i++) {
    const arg = rawExtraArgs[i];
    const isProcessName = arg === "--process-name" || arg === "-p";
    if (isProcessName) {
      hasProcessNameOverride = true;
      const value = rawExtraArgs[i + 1];
      if (typeof value === "string") {
        cleanedExtra.push(arg as string, value);
      }
      i++;
      continue;
    }
    if (typeof arg === "string") {
      cleanedExtra.push(arg);
    }
  }

  if (!processName && !hasProcessNameOverride) {
    return { kind: "missingProcessName" };
  }

  // "--no-color" is a top-level option; it must precede the `syslog` subcommand.
  // "--label" makes the CLI emit `[subsystem][category]` suffixes the parser reads.
  const baseArgs: string[] = ["--no-color", "syslog", "live", "--label"];
  if (!hasProcessNameOverride) {
    baseArgs.push("--process-name", processName as string);
  }

  return {
    kind: "ok",
    args: [...baseArgs, ...cleanedExtra],
    hasProcessNameOverride,
  };
}

export function shellQuote(value: string): string {
  if (value === "") return "''";
  if (/^[A-Za-z0-9_\-.,:/=@%+]+$/.test(value)) return value;
  return `'${value.replace(/'/g, "'\\''")}'`;
}

/**
 * When the user puts "--process-name NAME" / "-p NAME" into the extra args, we
 * honor it for the local image-name filter too. A null value (used to suppress
 * the default) yields undefined so we fall back to the provided `executableName`.
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

export function resolveDeviceLogBackend(): "off" | "osActivityDtMode" | "pymobiledevice3" {
  return getWorkspaceConfig("build.deviceLogStreamBackend") ?? "osActivityDtMode";
}

export function getDeviceLaunchEnvExtras(
  backend: "off" | "osActivityDtMode" | "pymobiledevice3",
): Record<string, string> {
  if (backend === "osActivityDtMode") {
    return { OS_ACTIVITY_DT_MODE: "enable" };
  }
  return {};
}

async function isPymobiledevice3Available(binaryPath: string): Promise<boolean> {
  try {
    await exec({ command: binaryPath, args: ["version"] });
    return true;
  } catch {
    return false;
  }
}
