import type { ExtensionContext } from "../common/commands";
import { ExtensionError } from "../common/errors";
import type { SimulatorDestination, SimulatorOS, SimulatorType } from "./types";

export async function getSimulatorByUdid(
  context: ExtensionContext,
  options: {
    udid: string;
  },
): Promise<SimulatorDestination> {
  const simulators = await context.destinationsManager.refreshSimulators();

  for (const simulator of simulators) {
    if (simulator.udid === options.udid) {
      return simulator;
    }
  }
  throw new ExtensionError("Simulator not found", { context: { udid: options.udid } });
}

/**
 * Parse the device type identifier to get the device type. Examples:
 *  - com.apple.CoreSimulator.SimDeviceType.Apple-Vision-Pro
 *  - com.apple.CoreSimulator.SimDeviceType.iPhone-8-Plus
 *  - com.apple.CoreSimulator.SimDeviceType.iPhone-SE-3rd-generation
 *  - com.apple.CoreSimulator.SimDeviceType.iPod-touch--7th-generation-
 *  - com.apple.CoreSimulator.SimDeviceType.iPad-Pro-11-inch-3rd-generation
 *  - com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-3rd-generation-4
 *  - com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-5-40mm
 */
export function parseDeviceTypeIdentifier(deviceTypeIdentifier: string): SimulatorType | null {
  const prefix = "com.apple.CoreSimulator.SimDeviceType.";
  if (!deviceTypeIdentifier?.startsWith(prefix)) {
    return null;
  }

  const deviceType = deviceTypeIdentifier.slice(prefix.length);
  if (!deviceType) {
    return null;
  }
  if (deviceType.startsWith("iPhone")) {
    return "iPhone";
  }
  if (deviceType.startsWith("iPad")) {
    return "iPad";
  }
  if (deviceType.startsWith("iPod")) {
    return "iPod";
  }
  if (deviceType.startsWith("Apple-TV")) {
    return "AppleTV";
  }
  if (deviceType.startsWith("Apple-Watch")) {
    return "AppleWatch";
  }
  if (deviceType.startsWith("Apple-Vision")) {
    return "AppleVision";
  }
  return null;
}

/**
 * Parse the simulator runtime to get the OS version. Examples:
 *  - com.apple.CoreSimulator.SimRuntime.xrOS-2-0
 *  - com.apple.CoreSimulator.SimRuntime.iOS-15-2
 *  - com.apple.CoreSimulator.SimRuntime.tvOS-18-0
 *  - com.apple.CoreSimulator.SimRuntime.watchOS-8-5
 */
export function parseSimulatorRuntime(runtime: string): {
  os: SimulatorOS;
  version: string;
} | null {
  const prefix = "com.apple.CoreSimulator.SimRuntime.";
  if (!runtime?.startsWith(prefix)) {
    return null;
  }

  // // com.apple.CoreSimulator.SimRuntime.iOS-15-2 -> 15.2
  // const rawOSVersion = runtime.split(".").slice(-1)[0];
  // const osVersion = rawOSVersion.replace(/^(\w+)-(\d+)-(\d+)$/, "$2.$3");

  // examples:
  // - xrOS-2-0 -> { os: "xrOS", version: "2.0" }
  // - iOS-15-2 -> { os: "iOS", version: "15.2" }
  // - tvOS-18-0 -> { os: "tvOS", version: "18.0" }
  // - watchOS-8-5 -> { os: "watchOS", version: "8.5" }
  const simRuntime = runtime.slice(prefix.length);
  if (!simRuntime) {
    return null;
  }

  const regex = /^(\w+)-(\d+)-(\d+)$/;
  const matches = simRuntime.match(regex);
  if (!matches) {
    return null;
  }

  const rawOs = matches[1] as string;
  const version = `${matches[2]}.${matches[3]}`;

  if (rawOs === "xrOS") {
    return { os: "xrOS", version };
  }
  if (rawOs === "iOS") {
    return { os: "iOS", version };
  }
  if (rawOs === "tvOS") {
    return { os: "tvOS", version };
  }
  if (rawOs === "watchOS") {
    return { os: "watchOS", version };
  }
  return null;
}
