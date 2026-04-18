import type { DeviceLogBackend } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";

export function resolveDeviceLogBackend(): DeviceLogBackend {
  return getWorkspaceConfig("build.deviceLogStreamBackend") ?? "off";
}

export function getDeviceLaunchEnvExtras(backend: DeviceLogBackend): Record<string, string> {
  if (backend === "osActivityDtMode") {
    return { OS_ACTIVITY_DT_MODE: "enable" };
  }
  return {};
}

export type Pymobiledevice3ArgsInput = {
  rawExtraArgs: (string | null)[];
  processName: string | undefined;
};

export type Pymobiledevice3ArgsResult =
  | {
      kind: "ok";
      args: string[];
      hasProcessNameOverride: boolean;
    }
  | { kind: "missingProcessName" };

/**
 * Build the argv for `pymobiledevice3 syslog live`, merging SweetPad's defaults
 * with user-supplied extras.
 *
 * SweetPad intentionally does NOT add "--match" / "--regex" any more — those
 * flags filter on the rendered line and are prone to both false positives
 * (framework chatter whose subsystem contains the bundle id) and false
 * negatives (logs from a custom `Logger(subsystem:)` that doesn't mention the
 * bundle id). Fine-grained filtering happens locally against the parsed
 * {@link SyslogEntry}; see `log-pipe.ts`.
 *
 * "--process-name" is still applied server-side as a cheap volume filter
 * (the relay only sends entries from processes matching the name). It does
 * NOT by itself exclude framework-emitted lines from *within* the app's
 * process — that's what the local image-name filter is for.
 *
 * "--no-color" is prepended as a top-level option so that piped output never
 * carries ANSI escape sequences; the parser tolerates them too, but stripping
 * at the source keeps the output channel clean if a user ever disables the
 * parser.
 *
 * Rules:
 * - "--process-name" / "-p" in extras fully replaces SweetPad's default.
 * - A "null" value after the flag suppresses SweetPad's default without
 *   adding a replacement.
 * - Any other args pass through in order.
 * - If the process name is missing AND no override was provided, returns
 *   `missingProcessName`.
 */
export function buildPymobiledevice3Args(input: Pymobiledevice3ArgsInput): Pymobiledevice3ArgsResult {
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

export function formatCommandLine(command: string, args: string[]): string {
  return [command, ...args].map(shellQuote).join(" ");
}

export function shellQuote(value: string): string {
  if (value === "") return "''";
  if (/^[A-Za-z0-9_\-.,:/=@%+]+$/.test(value)) return value;
  return `'${value.replace(/'/g, "'\\''")}'`;
}
