import type { ExtensionContext } from "../commands";
import { exec } from "../exec";
import { commonLogger } from "../logger";

/**
 * Represents a device from xcdevice list output
 * Note: xcdevice uses different field names than devicectl
 */
export type XcdeviceDevice = {
  identifier: string; // Different from devicectl identifier - this is the UDID
  modelCode: string; // Same as productType in devicectl (e.g., "iPhone10,6")
  name: string;
  operatingSystemVersion: string; // e.g., "16.7.12"
  platform: "com.apple.platform.iphoneos" | string;
};

type XcdeviceListOutput = XcdeviceDevice[];

/**
 * List devices using xcdevice command
 * This is a fallback for older devices that don't report OS version via devicectl
 */
export async function listDevicesWithXcdevice(context: ExtensionContext): Promise<XcdeviceDevice[]> {
  try {
    const stdout = await exec({
      command: "xcrun",
      args: ["xcdevice", "list"],
    });

    commonLogger.debug("xcdevice list output", { stdout });

    const devices: XcdeviceListOutput = JSON.parse(stdout);

    // Filter to only include iOS devices (not simulators)
    return devices.filter((device) => {
      // Only include physical iOS devices
      return device.platform === "com.apple.platform.iphoneos";
    });
  } catch (error) {
    commonLogger.error("Failed to list devices with xcdevice", { error });
    return [];
  }
}

/**
 * Create a lookup map from modelCode to OS version
 * This allows us to match devices by productType/modelCode
 */
export function createOsVersionLookup(devices: XcdeviceDevice[]): Map<string, string> {
  const lookup = new Map<string, string>();

  for (const device of devices) {
    if (device.modelCode && device.operatingSystemVersion) {
      // If multiple devices have the same modelCode, we keep the first one
      // In practice, this shouldn't matter as they should have the same OS version
      if (!lookup.has(device.modelCode)) {
        lookup.set(device.modelCode, device.operatingSystemVersion);
      }
    }
  }

  return lookup;
}

/**
 * Get OS version for a device by its productType (modelCode in xcdevice)
 */
export function getOsVersionForDevice(lookup: Map<string, string>, productType: string): string | undefined {
  return lookup.get(productType);
}

/**
 * Create a lookup map from modelCode to UDID (identifier in xcdevice)
 * This provides the correct UDID format for xcodebuild for older devices
 */
export function createUdidLookup(devices: XcdeviceDevice[]): Map<string, string> {
  const lookup = new Map<string, string>();

  for (const device of devices) {
    if (device.modelCode && device.identifier) {
      // If multiple devices have the same modelCode, we keep the first one
      // In practice, this shouldn't matter as they should have the same UDID
      if (!lookup.has(device.modelCode)) {
        lookup.set(device.modelCode, device.identifier);
      }
    }
  }

  return lookup;
}

/**
 * Get UDID for a device by its productType (modelCode in xcdevice)
 */
export function getUdidForDevice(lookup: Map<string, string>, productType: string): string | undefined {
  return lookup.get(productType);
}

/**
 * Create a lookup map from modelCode to device name
 * This provides the user-customized name for older devices where devicectl returns marketing name
 */
export function createNameLookup(devices: XcdeviceDevice[]): Map<string, string> {
  const lookup = new Map<string, string>();

  for (const device of devices) {
    if (device.modelCode && device.name) {
      // If multiple devices have the same modelCode, we keep the first one
      // In practice, this shouldn't matter as they should have different names
      if (!lookup.has(device.modelCode)) {
        lookup.set(device.modelCode, device.name);
      }
    }
  }

  return lookup;
}

/**
 * Get device name for a device by its productType (modelCode in xcdevice)
 */
export function getNameForDevice(lookup: Map<string, string>, productType: string): string | undefined {
  return lookup.get(productType);
}
