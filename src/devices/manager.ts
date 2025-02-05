import events from "node:events";
import type { ExtensionContext } from "../common/commands";
import { checkUnreachable } from "../common/types";
import { listDevices } from "../common/xcode/devicectl";
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
    const output = await listDevices(this.context);
    return output.result.devices
      .map((device) => {
        const deviceType = device.hardwareProperties.deviceType;
        if (deviceType === "appleWatch") {
          return new watchOSDeviceDestination(device);
        }
        if (deviceType === "iPhone" || deviceType === "iPad") {
          return new iOSDeviceDestination(device);
        }
        if (deviceType === "appleVision") {
          return new visionOSDeviceDestination(device);
        }
        if (deviceType === "appleTV") {
          return new tvOSDeviceDestination(device);
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
