import type { DeviceCtlDevice, DeviceCtlDeviceType } from "../common/xcode/devicectl";
import type { XcdeviceDevice } from "../common/xcode/xcdevice";
import type { DeviceRaw } from "./types";

/**
 * Determine the device type from whichever source has it. Returns null when neither
 * source has enough information to classify the device — such records cannot be
 * turned into a DeviceDestination and must be dropped.
 */
export function resolveDeviceType(raw: DeviceRaw): DeviceCtlDeviceType | null {
  const dcType = raw.devicectl?.hardwareProperties?.deviceType;
  if (dcType) {
    return dcType;
  }
  if (raw.xcdevice) {
    return inferDeviceTypeFromXcdevice(raw.xcdevice);
  }
  return null;
}

/**
 * Infer deviceType from an xcdevice record.
 *
 * xcdevice only exposes "platform" (a broad bucket) and "modelCode" (the specific
 * product identifier). For watchOS/tvOS/xrOS the platform alone is enough. For
 * iphoneos we disambiguate iPhone vs iPad via the modelCode prefix since both
 * share the same platform string.
 *
 * Returns null for unknown platforms so the caller can drop the record rather than
 * silently misclassify a future platform as iPhone.
 *
 * Examples:
 * - { platform: "com.apple.platform.iphoneos", modelCode: "iPhone14,2" } → "iPhone"
 * - { platform: "com.apple.platform.iphoneos", modelCode: "iPad14,5" }   → "iPad"
 * - { platform: "com.apple.platform.watchos",  modelCode: "Watch6,1" }   → "appleWatch"
 * - { platform: "com.apple.platform.appletvos", modelCode: "AppleTV11,1" } → "appleTV"
 * - { platform: "com.apple.platform.xros", modelCode: "RealityDevice14,1" } → "appleVision"
 */
function inferDeviceTypeFromXcdevice(xc: XcdeviceDevice): DeviceCtlDeviceType | null {
  switch (xc.platform) {
    case "com.apple.platform.watchos":
      return "appleWatch";
    case "com.apple.platform.appletvos":
      return "appleTV";
    case "com.apple.platform.xros":
      return "appleVision";
    case "com.apple.platform.iphoneos":
      return xc.modelCode?.startsWith("iPad") ? "iPad" : "iPhone";
    default:
      return null;
  }
}

/**
 * Merge devicectl and xcdevice outputs into a deduplicated list of source-record pairs.
 *
 * Scenario examples:
 *
 * - iOS 17 device, seen by both sources with matching UDID
 *   → 1 record with both devicectl and xcdevice set.
 *
 * - iOS 17 device, only in devicectl (xcdevice empty)
 *   → 1 record with just devicectl.
 *
 * - iOS 16 Wi-Fi device: devicectl list empty, xcdevice lists it
 *   → 1 record with just xcdevice.
 *
 * - iOS 16 USB device: devicectl returns it with empty hardwareProperties (no UDID),
 *   xcdevice also has it
 *   → devicectl entry can't be paired (no UDID) and has no deviceType → dropped.
 *     xcdevice entry stands alone → 1 record with just xcdevice.
 *
 * - Unpaired/locked device: xcdevice reports "available: false" / "error"
 *   → 1 record with just xcdevice (DeviceDestination will render as unavailable).
 */
export function mergeDeviceSources(
  devicectlDevices: DeviceCtlDevice[],
  xcdeviceDevices: XcdeviceDevice[],
): DeviceRaw[] {
  const xcByUdid = new Map<string, XcdeviceDevice>();
  for (const xc of xcdeviceDevices) {
    if (xc.identifier && xc.identifier.length > 0) {
      xcByUdid.set(xc.identifier.toLowerCase(), xc);
    }
  }

  const consumedXcUdids = new Set<string>();
  const result: DeviceRaw[] = [];

  for (const dc of devicectlDevices) {
    if (!dc.identifier) {
      continue;
    }

    const dcUdid = dc.hardwareProperties?.udid?.toLowerCase();
    const matchedXc = dcUdid ? xcByUdid.get(dcUdid) : undefined;

    // Rule 1: pair devicectl + xcdevice when UDIDs match (case-insensitive,
    // "devicectl.hardwareProperties.udid" ↔ "xcdevice.identifier").
    if (matchedXc && dcUdid) {
      consumedXcUdids.add(dcUdid);
      result.push({ devicectl: dc, xcdevice: matchedXc });
      continue;
    }

    // Rule 2: devicectl entries with no "deviceType" AND no xcdevice match are
    // dropped — no way to pick a DeviceDestination subclass for them.
    if (!dc.hardwareProperties?.deviceType) {
      continue;
    }
    result.push({ devicectl: dc });
  }

  // Rule 3: xcdevice entries with no devicectl counterpart become standalone
  // records — the iOS <= 16 recovery path.
  for (const xc of xcdeviceDevices) {
    const udid = xc.identifier?.toLowerCase();
    if (!udid || consumedXcUdids.has(udid)) {
      continue;
    }
    result.push({ xcdevice: xc });
  }

  return result;
}
