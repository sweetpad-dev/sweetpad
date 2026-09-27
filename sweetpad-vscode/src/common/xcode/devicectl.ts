import type { DevicectlAppProcess, DevicectlDevice } from "@sweetpad/native";
import type * as vscode from "vscode";

import { exec } from "../exec";
import { readTextFile, tempFilePath } from "../files";
import { commonLogger } from "../logger";

/**
 * A physical device as devicectl lists it. The addon's "parseDevicectlDevices" reads
 * both JSON shapes devicectl writes and leaves out the simulators Xcode 27 lists beside
 * the devices, the same way the CLI does.
 */
export type { DevicectlAppProcess, DevicectlDevice };

export type DeviceCtlTunnelState = "disconnected" | "connected" | "unavailable";

export type DeviceCtlDeviceType = "iPhone" | "iPad" | "appleWatch" | "appleTV" | "appleVision" | "realityDevice";

const DEVICE_TYPES: ReadonlySet<string> = new Set<DeviceCtlDeviceType>([
  "iPhone",
  "iPad",
  "appleWatch",
  "appleTV",
  "appleVision",
  "realityDevice",
]);

const TUNNEL_STATES: ReadonlySet<string> = new Set<DeviceCtlTunnelState>(["disconnected", "connected", "unavailable"]);

/** The device's type, or undefined when devicectl leaves it out or names one this code doesn't know. */
export function deviceType(device: DevicectlDevice): DeviceCtlDeviceType | undefined {
  const type = device.deviceType;
  return type && DEVICE_TYPES.has(type) ? (type as DeviceCtlDeviceType) : undefined;
}

/** Reachability, e.g. "connected". */
export function deviceTunnelState(device: DevicectlDevice): DeviceCtlTunnelState | undefined {
  const state = device.connection;
  return state && TUNNEL_STATES.has(state) ? (state as DeviceCtlTunnelState) : undefined;
}

/**
 * Run "devicectl list devices" and return the JSON it wrote, for the addon's
 * "parseDevicectlDevices" to read.
 */
export async function listDevicesJson(vscodeContext: vscode.ExtensionContext): Promise<string> {
  await using tmpPath = await tempFilePath(vscodeContext, {
    prefix: "devices",
  });

  const devicesStdout = await exec({
    command: "xcrun",
    args: ["devicectl", "list", "devices", "--json-output", tmpPath.path, "--timeout", "10"],
    cwd: null,
  });
  commonLogger.debug("Stdout devicectl list devices", { stdout: devicesStdout });

  return await readTextFile(tmpPath.path);
}

/**
 * Run "devicectl device info processes" on a device and return the JSON it wrote, for
 * the addon's "devicectlAppProcesses" to read.
 */
export async function getRunningProcessesJson(
  vscodeContext: vscode.ExtensionContext,
  options: {
    deviceId: string;
  },
): Promise<string> {
  await using tmpPath = await tempFilePath(vscodeContext, {
    prefix: "processes",
  });
  // xcrun devicectl device info processes -d 2782A5CE-797F-4EB9-BDF1-14AE4425C406 --json-output <path>
  await exec({
    command: "xcrun",
    args: ["devicectl", "device", "info", "processes", "-d", options.deviceId, "--json-output", tmpPath.path],
    cwd: null,
  });

  return await readTextFile(tmpPath.path);
}

export async function pairDevice(options: { deviceId: string }): Promise<void> {
  // xcrun devicectl manage pair --device 00008110-000559182E90401E
  await exec({
    command: "xcrun",
    args: ["devicectl", "manage", "pair", "--device", options.deviceId],
    cwd: null,
  });
}
