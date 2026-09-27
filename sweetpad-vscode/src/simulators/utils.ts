import { ExtensionError } from "../common/errors";
import type { DestinationsManager } from "../destination/manager";
import type { SimulatorDestination, SimulatorType } from "./types";

export async function getSimulatorByUdid(
  destinationsManager: DestinationsManager,
  options: {
    udid: string;
  },
): Promise<SimulatorDestination> {
  const simulators = await destinationsManager.refreshSimulators();

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
