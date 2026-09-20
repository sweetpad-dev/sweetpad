import type * as vscode from "vscode";

import { exec } from "../exec";
import { readJsonFile, tempFilePath } from "../files";
import { commonLogger } from "../logger";

type DeviceCtlListCommandOutput = {
  result: {
    devices: DeviceCtlDevice[];
  };
};

/**
 * A device as devicectl reports it, in either of the two shapes it emits.
 *
 * Through "jsonVersion" 4 a device carried "hardwareProperties" /
 * "deviceProperties" / "connectionProperties". Version 5 (Xcode 27) adds
 * "properties", which supersedes all three; devicectl also attaches a
 * "_deprecationNotice" saying the old trio will be removed in a future release.
 * Xcode 27 still fills in both, so the trio is optional rather than gone, and
 * the accessors below read "properties" first.
 *
 * Reach for the accessors rather than these fields — "deviceUdid", "deviceName",
 * "deviceOsVersion" and friends are the only places that know which shape is in
 * front of them.
 */
export type DeviceCtlDevice = {
  capabilities: DeviceCtlDeviceCapability[];
  connectionProperties?: DeviceCtlConnectionProperties;
  deviceProperties?: DeviceCtlDeviceProperties;
  hardwareProperties?: DeviceCtlHardwareProperties;
  properties?: DeviceCtlProperties;
  identifier: string;
  visibilityClass: "default";
};

export type DeviceCtlTunnelState = "disconnected" | "connected" | "unavailable";

/** The "jsonVersion" 5 dictionary that replaces the three deprecated ones. */
type DeviceCtlProperties = {
  connection?: DeviceCtlConnectionSection;
  hardware?: DeviceCtlHardwareSection;
  software?: DeviceCtlSoftwareSection;
  state?: DeviceCtlStateSection;
};

type DeviceCtlConnectionSection = {
  authenticationType?: string;
  /** Core Foundation absolute time — seconds since 2001-01-01, not an ISO string. */
  lastConnectionDate?: number;
  pairingState?: "paired" | "unsupported";
  /** Version 5's spelling of "connectionProperties.tunnelState". */
  state?: DeviceCtlTunnelState;
  transportType?: "localNetwork" | "wired" | "sameMachine";
};

type DeviceCtlHardwareSection = {
  deviceType?: DeviceCtlDeviceType;
  marketingName?: string;
  platform?: "iOS";
  productType?: string;
  reality?: "physical" | "simulated";
  udid?: string;
};

type DeviceCtlSoftwareSection = {
  /** An object here, where the deprecated "osVersionNumber" was the string itself. */
  osVersionNumber?: { components?: number[]; stringValue?: string };
};

type DeviceCtlStateSection = {
  bootState?: string;
  name?: string;
  visibilityClass?: string;
};

type DeviceCtlConnectionProperties = {
  authenticationType?: "manualPairing";
  isMobileDeviceOnly?: boolean;
  lastConnectionDate?: string;
  pairingState: "paired" | "unsupported";
  potentialHostnames?: string[];
  transportType?: "localNetwork" | "wired";
  tunnelState?: DeviceCtlTunnelState;
  tunnelTransportProtocol?: "tcp";
};

type DeviceCtlCpuType = {
  name: "arm64e" | "arm64" | "arm64_32";
  subType: number;
  type: number;
};

type DeviceCtlDeviceProperties = {
  bootedFromSnapshot?: boolean;
  bootedSnapshotName?: string;
  ddiServicesAvailable?: boolean;
  developerModeStatus?: "enabled";
  hasInternalOSBuild?: boolean;
  name?: string;
  osBuildUpdate?: string;
  osVersionNumber?: string;
  rootFileSystemIsWritable?: boolean;
};

export type DeviceCtlDeviceType = "iPhone" | "iPad" | "appleWatch" | "appleTV" | "appleVision" | "realityDevice";

/**
 * All fields are optional because devicectl returns "hardwareProperties": {} for
 * some iOS <= 16 devices connected via USB (see sweetpad-dev/sweetpad#223). Callers
 * must handle missing deviceType / platform / udid.
 */
type DeviceCtlHardwareProperties = {
  cpuType?: DeviceCtlCpuType;
  deviceType?: DeviceCtlDeviceType;
  ecid?: number;
  hardwareModel?: string;
  internalStorageCapacity?: number;
  isProductionFused?: boolean;
  marketingName?: string;
  platform?: "iOS";
  productType?: string;
  reality?: "physical";
  serialNumber?: string;
  supportedCPUTypes?: DeviceCtlCpuType[];
  supportedDeviceFamilies?: number[];
  thinningProductType?: string;
  udid?: string;
};

type DeviceCtlDeviceCapability = {
  name: string;
  featureIdentifier: string;
};

/**
 * Seconds between the Unix epoch and Core Foundation's 2001-01-01 reference date.
 */
const CF_EPOCH_OFFSET_SECONDS = 978_307_200;

/** Hex UDID, e.g. "00008110-001234567890001E". */
export function deviceUdid(device: DeviceCtlDevice): string | undefined {
  return device.properties?.hardware?.udid ?? device.hardwareProperties?.udid;
}

/** The user-facing name, e.g. "John's iPhone". */
export function deviceName(device: DeviceCtlDevice): string | undefined {
  return device.properties?.state?.name ?? device.deviceProperties?.name;
}

/** The product's marketing name, e.g. "iPhone 15 Pro". */
export function deviceMarketingName(device: DeviceCtlDevice): string | undefined {
  return device.properties?.hardware?.marketingName ?? device.hardwareProperties?.marketingName;
}

/** The model code, e.g. "iPhone15,2". */
export function deviceProductType(device: DeviceCtlDevice): string | undefined {
  return device.properties?.hardware?.productType ?? device.hardwareProperties?.productType;
}

export function deviceType(device: DeviceCtlDevice): DeviceCtlDeviceType | undefined {
  return device.properties?.hardware?.deviceType ?? device.hardwareProperties?.deviceType;
}

/** Plain version string, e.g. "17.0". */
export function deviceOsVersion(device: DeviceCtlDevice): string | undefined {
  return device.properties?.software?.osVersionNumber?.stringValue ?? device.deviceProperties?.osVersionNumber;
}

/** Reachability, e.g. "connected". */
export function deviceTunnelState(device: DeviceCtlDevice): DeviceCtlTunnelState | undefined {
  return device.properties?.connection?.state ?? device.connectionProperties?.tunnelState;
}

/**
 * When the device was last seen. Null when devicectl omits it and when the value
 * doesn't parse, so callers can sort "unknown" as oldest.
 */
export function deviceLastConnectionDate(device: DeviceCtlDevice): Date | null {
  const referenceSeconds = device.properties?.connection?.lastConnectionDate;
  if (typeof referenceSeconds === "number" && Number.isFinite(referenceSeconds)) {
    return new Date((referenceSeconds + CF_EPOCH_OFFSET_SECONDS) * 1000);
  }
  const iso = device.connectionProperties?.lastConnectionDate;
  if (!iso) {
    return null;
  }
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? null : date;
}

export async function listDevices(vscodeContext: vscode.ExtensionContext): Promise<DeviceCtlListCommandOutput> {
  await using tmpPath = await tempFilePath(vscodeContext, {
    prefix: "devices",
  });

  const devicesStdout = await exec({
    command: "xcrun",
    args: ["devicectl", "list", "devices", "--json-output", tmpPath.path, "--timeout", "10"],
    cwd: null,
  });
  commonLogger.debug("Stdout devicectl list devices", { stdout: devicesStdout });

  return await readJsonFile<DeviceCtlListCommandOutput>(tmpPath.path);
}

export type DeviceCtlProcessResult = {
  result: {
    runningProcesses: DeviceCtlProcess[];
  };
};

export type DeviceCtlProcess = {
  executable?: string; // Ex: file:///private/var/containers/Bundle/Application/183E1862-A6F2-4060-AEEF-16F61C88F91E/terminal23.app/terminal23
  processIdentifier: number; // Ex: 1234
};

export async function getRunningProcesses(
  vscodeContext: vscode.ExtensionContext,
  options: {
    deviceId: string;
  },
): Promise<DeviceCtlProcessResult> {
  await using tmpPath = await tempFilePath(vscodeContext, {
    prefix: "processes",
  });
  // xcrun devicectl device info processes -d 2782A5CE-797F-4EB9-BDF1-14AE4425C406 --json-output <path>
  await exec({
    command: "xcrun",
    args: ["devicectl", "device", "info", "processes", "-d", options.deviceId, "--json-output", tmpPath.path],
    cwd: null,
  });

  return await readJsonFile<DeviceCtlProcessResult>(tmpPath.path);
}

export async function pairDevice(options: { deviceId: string }): Promise<void> {
  // xcrun devicectl manage pair --device 00008110-000559182E90401E
  await exec({
    command: "xcrun",
    args: ["devicectl", "manage", "pair", "--device", options.deviceId],
    cwd: null,
  });
}
