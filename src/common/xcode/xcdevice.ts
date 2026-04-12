import type { ExtensionContext } from "../commands";
import { exec } from "../exec";
import { commonLogger } from "../logger";

/**
 * Represents a device from "xcrun xcdevice list" output.
 *
 * xcdevice returns physical devices (and simulators) across all Apple platforms.
 * Fields come from Xcode's own device database and are more reliable than devicectl
 * for iOS <= 16 devices, which devicectl either reports with empty hardwareProperties
 * or omits entirely.
 */
export type XcdeviceDevice = {
  /** UDID in legacy format (e.g. "00008110-001234567890001E"). Same value as DeviceCtlDevice.hardwareProperties.udid when both sources report the device. */
  identifier: string;
  /** e.g. "iPhone15,2" — same as DeviceCtlDevice.hardwareProperties.productType. */
  modelCode: string;
  /** User-customized device name, or marketing name if never customized. */
  name: string;
  /** e.g. "16.7.12". May be an empty string for unavailable devices. */
  operatingSystemVersion: string;
  /** "com.apple.platform.iphoneos" | "com.apple.platform.watchos" | "com.apple.platform.appletvos" | "com.apple.platform.xros" | ... */
  platform: string;
  /** True for iOS/watchOS simulators. We filter these out. */
  simulator?: boolean;
  /** False when the device is not currently reachable/paired. Still listed, just with degraded info. */
  available?: boolean;
  /** Transport Xcode believes it would use. Note: Xcode 15+ lies here for iOS 17+ wireless devices. */
  interface?: "usb" | "network" | string;
  /** CPU architecture (e.g. "arm64e"). Not always present. */
  architecture?: string;
  /** Friendly model name (e.g. "iPhone 13 Pro"). Not always present. */
  modelName?: string;
  /** Present on unavailable entries. Common codes: -9 not paired, -10 locked, -14 dev-mode disabled. */
  error?: {
    code: number;
    domain?: string;
    failureReason?: string;
    description?: string;
  };
};

type XcdeviceListOutput = XcdeviceDevice[];

/**
 * Platform strings we accept as physical Apple devices. Anything else (simulators,
 * macOS, driverkit, etc.) is filtered out.
 */
const SUPPORTED_PLATFORMS = new Set([
  "com.apple.platform.iphoneos",
  "com.apple.platform.watchos",
  "com.apple.platform.appletvos",
  "com.apple.platform.xros",
]);

/**
 * List devices using "xcrun xcdevice list".
 *
 * Retains entries with "available: false" or an "error" payload — downstream code
 * (DeviceDestinationBase.state) derives "unavailable" from those so the device still shows
 * in the tree, just marked disconnected.
 */
export async function listDevicesWithXcdevice(context: ExtensionContext): Promise<XcdeviceDevice[]> {
  try {
    const stdout = await exec({
      command: "xcrun",
      args: ["xcdevice", "list"],
    });

    commonLogger.debug("xcdevice list output", { stdout });

    const devices: XcdeviceListOutput = JSON.parse(stdout);

    return devices.filter((device) => {
      if (device.simulator === true) {
        return false;
      }
      return SUPPORTED_PLATFORMS.has(device.platform);
    });
  } catch (error) {
    commonLogger.error("Failed to list devices with xcdevice", { error });
    return [];
  }
}
