import events from "node:events";

import type { Logger } from "../logger/types";
import { checkUnreachable } from "../types";
import type { WorkspaceRoot } from "../workspace-root";
import { listDevices } from "../xcode/devicectl";
import { listDevicesWithXcdevice } from "../xcode/xcdevice";
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
  private logger: Logger;
  private workspaceRoot: WorkspaceRoot;
  private emitter = new events.EventEmitter<DeviceManagerEventTypes>();

  public failed: "unknown" | "no-devicectl" | null = null;

  constructor(options: { logger: Logger; workspaceRoot: WorkspaceRoot }) {
    this.logger = options.logger;
    this.workspaceRoot = options.workspaceRoot;
  }

  on(event: "updated", listener: () => void): void {
    this.emitter.on(event, listener);
  }

  private async fetchDevices(): Promise<{ devices: DeviceDestination[]; devicectlError: unknown }> {
    // Resolve workspace bits lazily — throws here (not at boot) if no folder is open.
    const cwd = this.workspaceRoot.getPath();
    const storagePath = await this.workspaceRoot.getStoragePath();

    // Run both sources in parallel; degrade rather than fail if one source errors.
    // The iOS <= 16 recovery path relies on xcdevice — we must not drop xcdevice
    // results just because devicectl (ENOENT on old Xcode, sandboxed env) blew up.
    const [devicectlResult, xcdeviceResult] = await Promise.allSettled([
      listDevices({ storagePath: storagePath, cwd: cwd, logger: this.logger }),
      listDevicesWithXcdevice({ cwd: cwd, logger: this.logger }),
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
