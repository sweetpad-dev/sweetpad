import type { ExtensionContext } from "../commands";
import { exec } from "../exec";
import { readJsonFile, tempFilePath } from "../files";
import { commonLogger } from "../logger";

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
  authenticationType: "manualPairing";
  isMobileDeviceOnly: boolean;
  lastConnectionDate: string;
  pairingState: "paired";
  potentialHostnames: string[];
  transportType: "localNetwork" | "wired";
  tunnelState: "disconnected" | "connected" | "unavailable";
  tunnelTransportProtocol: "tcp";
};

type DeviceCtlCpuType = {
  name: "arm64e" | "arm64" | "arm64_32";
  subType: number;
  type: number;
};

type DeviceCtlDeviceProperties = {
  bootedFromSnapshot: boolean;
  bootedSnapshotName: string;
  ddiServicesAvailable: boolean;
  developerModeStatus: "enabled";
  hasInternalOSBuild: boolean;
  name: string;
  osBuildUpdate: string;
  osVersionNumber: string;
  rootFileSystemIsWritable: boolean;
};

export type DeviceCtlDeviceType = "iPhone" | "iPad" | "appleWatch";

type DeviceCtlHardwareProperties = {
  cpuType: DeviceCtlCpuType;
  deviceType: DeviceCtlDeviceType;
  ecid: number;
  hardwareModel: string;
  internalStorageCapacity: number;
  isProductionFused: boolean;
  marketingName: string;
  platform: "iOS";
  productType: "iPhone13,4" | "iPhone15,3";
  reality: "physical";
  serialNumber: string;
  supportedCPUTypes: DeviceCtlCpuType[];
  supportedDeviceFamilies: number[];
  thinningProductType: "iPhone15,3";
  udid: string;
};

type DeviceCtlDeviceCapability = {
  name: string;
  featureIdentifier: string;
};

export async function listDevices(context: ExtensionContext): Promise<DeviceCtlListCommandOutput> {
  await using tmpPath = await tempFilePath(context, {
    prefix: "devices",
  });

  const devicesStdout = await exec({
    command: "xcrun",
    args: ["devicectl", "list", "devices", "--json-output", tmpPath.path, "--timeout", "10"],
  });
  commonLogger.debug("Stdout devicectl list devices", { stdout: devicesStdout });

  return await readJsonFile<DeviceCtlListCommandOutput>(tmpPath.path);
}
