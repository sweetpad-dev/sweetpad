import { exec } from "../exec";
import { readJsonFile, tempFilePath } from "../files";
import type { Logger } from "../logger/types";

type DeviceCtlListCommandOutput = {
  result: {
    devices: DeviceCtlDevice[];
  };
};

export type DeviceCtlDevice = {
  capabilities: DeviceCtlDeviceCapability[];
  connectionProperties: DeviceCtlConnectionProperties;
  deviceProperties: DeviceCtlDeviceProperties;
  hardwareProperties: DeviceCtlHardwareProperties;
  identifier: string;
  visibilityClass: "default";
};

type DeviceCtlConnectionProperties = {
  authenticationType?: "manualPairing";
  isMobileDeviceOnly?: boolean;
  lastConnectionDate?: string;
  pairingState: "paired" | "unsupported";
  potentialHostnames?: string[];
  transportType?: "localNetwork" | "wired";
  tunnelState: "disconnected" | "connected" | "unavailable";
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

export async function listDevices(options: {
  storagePath: string;
  cwd: string;
  logger: Logger;
}): Promise<DeviceCtlListCommandOutput> {
  await using tmpPath = await tempFilePath(options.storagePath, {
    prefix: "devices",
  });

  const devicesStdout = await exec({
    command: "xcrun",
    args: ["devicectl", "list", "devices", "--json-output", tmpPath.path, "--timeout", "10"],
    cwd: options.cwd,
    logger: options.logger,
  });
  options.logger.debug("Stdout devicectl list devices", { stdout: devicesStdout });

  return await readJsonFile<DeviceCtlListCommandOutput>(tmpPath.path);
}

export type DeviceCtlProcessResult = {
  result: {
    runningProcesses: DeviceCtlProcess[];
  };
};

export type DeviceCtlProcess = {
  executable?: string;
  processIdentifier: number;
};

export async function getRunningProcesses(options: {
  storagePath: string;
  cwd: string;
  deviceId: string;
  logger: Logger;
}): Promise<DeviceCtlProcessResult> {
  await using tmpPath = await tempFilePath(options.storagePath, {
    prefix: "processes",
  });
  // xcrun devicectl device info processes -d 2782A5CE-797F-4EB9-BDF1-14AE4425C406 --json-output <path>
  await exec({
    command: "xcrun",
    args: ["devicectl", "device", "info", "processes", "-d", options.deviceId, "--json-output", tmpPath.path],
    cwd: options.cwd,
    logger: options.logger,
  });

  return await readJsonFile<DeviceCtlProcessResult>(tmpPath.path);
}

export async function pairDevice(options: { deviceId: string; cwd: string; logger: Logger }): Promise<void> {
  // xcrun devicectl manage pair --device 00008110-000559182E90401E
  await exec({
    command: "xcrun",
    args: ["devicectl", "manage", "pair", "--device", options.deviceId],
    cwd: options.cwd,
    logger: options.logger,
  });
}
