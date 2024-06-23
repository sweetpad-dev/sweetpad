import { SimulatorTreeItem } from "../../simulators/tree";
import { ExtensionContext } from "../commands";
import { exec } from "../exec";
import { readFile, readJsonFile, removeFile, tempFilePath } from "../files";
import { commonLogger } from "../logger";

type DeviceCtlListCommandOutput = {
  result: {
    devices: DeviceCtlDevice[];
  };
};

type DeviceCtlDevice = {
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

type DeviceCtlHardwareProperties = {
  cpuType: DeviceCtlCpuType;
  deviceType: "iPhone";
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

export class IosDevice {
  constructor(public device: DeviceCtlDevice) {
    this.device = device;
  }

  get udid() {
    return this.device.hardwareProperties.udid;
  }

  get label() {
    return `${this.device.deviceProperties.name} (${this.device.deviceProperties.osVersionNumber})`;
  }

  get state(): "connected" | "disconnected" | "unavailable" {
    return this.device.connectionProperties.tunnelState;
  }
}

export async function listDevices(context: ExtensionContext): Promise<IosDevice[]> {
  await using tmpPath = await tempFilePath(context, {
    prefix: "devices",
  });

  const devicesStdout = await exec({
    command: "xcrun",
    args: ["devicectl", "list", "devices", "--json-output", tmpPath.path, "--timeout", "10"],
  });
  commonLogger.debug("Stdout devicectl list devices", { stdout: devicesStdout });

  const output = await readJsonFile<DeviceCtlListCommandOutput>(tmpPath.path);
  return output.result.devices.map((device) => new IosDevice(device));
}
