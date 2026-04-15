import events from "node:events";
import type { ExtensionContext } from "../common/commands";
import { checkUnreachable } from "../common/types";
import { listDevices } from "../common/xcode/devicectl";
import { listDevicesWithXcdevice } from "../common/xcode/xcdevice";
import { mergeDeviceSources, resolveDeviceType } from "./merge";
import {
  type DeviceDestination,
  type DeviceRaw,
  iOSDeviceDestination,
  tvOSDeviceDestination,
  visionOSDeviceDestination,
  watchOSDeviceDestination,
} from "./types";

type DeviceManagerEventTypes = {
  updated: [];
};

function buildDeviceDestination(raw: DeviceRaw): DeviceDestination | null {
  const deviceType = resolveDeviceType(raw);
  if (!deviceType) {
    return null;
  }
  switch (deviceType) {
    case "appleWatch":
      return new watchOSDeviceDestination(raw);
    case "iPhone":
    case "iPad":
      return new iOSDeviceDestination(raw);
    case "appleVision":
    case "realityDevice":
      return new visionOSDeviceDestination(raw);
    case "appleTV":
      return new tvOSDeviceDestination(raw);
    default:
      checkUnreachable(deviceType);
      return null;
  }
}

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

  private async fetchDevices(): Promise<{ devices: DeviceDestination[]; devicectlError: unknown }> {
    // Run both sources in parallel; degrade rather than fail if one source errors.
    // The iOS <= 16 recovery path relies on xcdevice — we must not drop xcdevice
    // results just because devicectl (ENOENT on old Xcode, sandboxed env) blew up.
    const [devicectlResult, xcdeviceResult] = await Promise.allSettled([
      listDevices(this.context),
      listDevicesWithXcdevice(this.context),
    ]);

    const devicectlDevices = devicectlResult.status === "fulfilled" ? devicectlResult.value.result.devices : [];
    const xcdeviceList = xcdeviceResult.status === "fulfilled" ? xcdeviceResult.value : [];

    const merged = mergeDeviceSources(devicectlDevices, xcdeviceList);
    const devices = merged.map(buildDeviceDestination).filter((d) => d !== null);

    return {
      devices,
      devicectlError: devicectlResult.status === "rejected" ? devicectlResult.reason : null,
    };
  }

  async refresh(): Promise<DeviceDestination[]> {
    this.failed = null;
    try {
      const { devices, devicectlError } = await this.fetchDevices();
      this.cache = devices;
      if (devicectlError) {
        // Only surface devicectl failure when we have nothing to show — otherwise
        // xcdevice recovered some devices and the user shouldn't see an error banner.
        if (devices.length === 0) {
          const code = (devicectlError as any)?.error?.code;
          this.failed = code === "ENOENT" ? "no-devicectl" : "unknown";
        }
      }
    } catch (error) {
      this.failed = "unknown";
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
