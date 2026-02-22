import events from "node:events";
import type { ExtensionContext } from "../common/commands";
import { checkUnreachable } from "../common/types";
import { listDevices } from "../common/xcode/devicectl";
import {
  createNameLookup,
  createOsVersionLookup,
  createUdidLookup,
  getNameForDevice,
  getOsVersionForDevice,
  getUdidForDevice,
  listDevicesWithXcdevice,
} from "../common/xcode/xcdevice";
import {
  type DeviceDestination,
  iOSDeviceDestination,
  tvOSDeviceDestination,
  visionOSDeviceDestination,
  watchOSDeviceDestination,
} from "./types";

type DeviceManagerEventTypes = {
  updated: [];
};

export class DevicesManager {
  private cache: DeviceDestination[] | undefined = undefined;
  private _context: ExtensionContext | undefined = undefined;
  private emitter = new events.EventEmitter<DeviceManagerEventTypes>();

  public failed: "unknown" | "no-devicectl" | null = null;

  set context(context: ExtensionContext) {
    this._context = context;
  }

  on(event: "updated", listener: () => void): void {
    this.emitter.on(event, listener);
  }

  get context(): ExtensionContext {
    if (!this._context) {
      throw new Error("Context is not set");
    }
    return this._context;
  }

  private async fetchDevices(): Promise<DeviceDestination[]> {
    // Fetch devices from both sources in parallel
    const [output, xcdeviceList] = await Promise.all([
      listDevices(this.context),
      listDevicesWithXcdevice(this.context),
    ]);

    // Create lookup maps from modelCode to OS version, UDID, and name for fallback
    const osVersionLookup = createOsVersionLookup(xcdeviceList);
    const udidLookup = createUdidLookup(xcdeviceList);
    const nameLookup = createNameLookup(xcdeviceList);

    return output.result.devices
      .filter((device) => {
        // Filter out devices without required fields
        if (!device.identifier) {
          return false;
        }
        if (!device.hardwareProperties?.deviceType) {
          return false;
        }
        return true;
      })
      .map((device) => {
        // Get OS version from devicectl or fallback to xcdevice
        let osVersionNumber = device.deviceProperties.osVersionNumber;
        if (!osVersionNumber && device.hardwareProperties.productType) {
          const xcdeviceVersion = getOsVersionForDevice(osVersionLookup, device.hardwareProperties.productType);
          if (xcdeviceVersion) {
            osVersionNumber = xcdeviceVersion;
          }
        }

        // Get UDID from devicectl or fallback to xcdevice (for older devices)
        // xcdevice provides the correct UDID format for xcodebuild
        let udid = device.hardwareProperties.udid;
        if (!udid && device.hardwareProperties.productType) {
          const xcdeviceUdid = getUdidForDevice(udidLookup, device.hardwareProperties.productType);
          if (xcdeviceUdid) {
            udid = xcdeviceUdid;
          }
        }

        // Get device name from devicectl or fallback to xcdevice (for iOS < 17 devices)
        // devicectl may return marketing name instead of user-customized name for older devices
        let deviceName = device.deviceProperties.name;
        if (
          (!deviceName || deviceName === device.hardwareProperties.marketingName) &&
          device.hardwareProperties.productType
        ) {
          const xcdeviceName = getNameForDevice(nameLookup, device.hardwareProperties.productType);
          if (xcdeviceName) {
            deviceName = xcdeviceName;
          }
        }

        // Apply safe defaults for missing data
        const safeDevice = {
          ...device,
          hardwareProperties: {
            ...device.hardwareProperties,
            udid: udid ?? device.identifier,
            marketingName: device.hardwareProperties.marketingName,
            productType: device.hardwareProperties.productType ?? "Unknown",
          },
          deviceProperties: {
            ...device.deviceProperties,
            name: deviceName,
            osVersionNumber: osVersionNumber,
          },
        };

        const deviceType = safeDevice.hardwareProperties.deviceType;
        if (deviceType === "appleWatch") {
          return new watchOSDeviceDestination(safeDevice);
        }
        if (deviceType === "iPhone" || deviceType === "iPad") {
          return new iOSDeviceDestination(safeDevice);
        }
        if (deviceType === "appleVision" || deviceType === "realityDevice") {
          return new visionOSDeviceDestination(safeDevice);
        }
        if (deviceType === "appleTV") {
          return new tvOSDeviceDestination(safeDevice);
        }
        checkUnreachable(deviceType);
        return null; // Unsupported device type
      })
      .filter((device) => device !== null);
  }

  async refresh(): Promise<DeviceDestination[]> {
    this.failed = null;
    try {
      this.cache = await this.fetchDevices();
    } catch (error: any) {
      if (error?.error?.code === "ENOENT") {
        this.failed = "no-devicectl";
      } else {
        this.failed = "unknown";
      }
      this.cache = [];
    }
    this.emitter.emit("updated");
    return this.cache;
  }

  async getDevices(options?: { refresh?: boolean }): Promise<DeviceDestination[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh();
    }
    return this.cache;
  }
}
