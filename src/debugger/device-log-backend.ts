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
      hasRegexOverride: boolean;
    }
  | { kind: "missingProcessName" };

export function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function buildDefaultPymobiledevice3Regex(processName: string): string {
  const escaped = escapeRegex(processName);
  return `${escaped}\\{${escaped}(\\.debug\\.dylib)?\\}\\[`;
}

/**
 * Merge SweetPad's default `pymobiledevice3 syslog live` arguments with user-supplied extras.
 *
 * Rules:
 * - `--process-name`/`-p` and `--regex`/`-e` in extras fully replace SweetPad's defaults.
 * - A null value after either flag suppresses SweetPad's default without adding a replacement.
 * - Any other args are passed through in order.
 * - If the process name is missing AND no override was provided, returns `missingProcessName`.
 */
export function buildPymobiledevice3Args(input: Pymobiledevice3ArgsInput): Pymobiledevice3ArgsResult {
  const { rawExtraArgs, processName } = input;

  let hasProcessNameOverride = false;
  let hasRegexOverride = false;
  let overriddenProcessName: string | undefined;
  const cleanedExtra: string[] = [];

  for (let i = 0; i < rawExtraArgs.length; i++) {
    const arg = rawExtraArgs[i];
    const isProcessName = arg === "--process-name" || arg === "-p";
    const isRegex = arg === "--regex" || arg === "-e";
    if (isProcessName || isRegex) {
      if (isProcessName) hasProcessNameOverride = true;
      else hasRegexOverride = true;
      const value = rawExtraArgs[i + 1];
      if (typeof value === "string") {
        if (isProcessName) {
          overriddenProcessName = value;
        }
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

  const baseArgs: string[] = ["syslog", "live", "--label"];
  if (!hasProcessNameOverride) {
    baseArgs.push("--process-name", processName as string);
  }
  const regexProcessName = overriddenProcessName ?? processName;
  if (!hasRegexOverride && regexProcessName) {
    baseArgs.push("--regex", buildDefaultPymobiledevice3Regex(regexProcessName));
  }

  return {
    kind: "ok",
    args: [...baseArgs, ...cleanedExtra],
    hasProcessNameOverride,
    hasRegexOverride,
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
