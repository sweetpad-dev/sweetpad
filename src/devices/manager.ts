import events from "node:events";
import type { ExtensionContext } from "../common/commands";
import { listDevices } from "../common/xcode/devicectl";
import { iOSDeviceDestination } from "./types";

type DeviceManagerEventTypes = {
  updated: [];
};

export class DevicesManager {
  private cache: iOSDeviceDestination[] | undefined = undefined;
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

  private async fetchDevices(): Promise<iOSDeviceDestination[]> {
    const output = await listDevices(this.context);
    return output.result.devices.map((device) => new iOSDeviceDestination(device));
  }

  async refresh(): Promise<iOSDeviceDestination[]> {
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

  async getDevices(options?: { refresh?: boolean }): Promise<iOSDeviceDestination[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh();
    }
    return this.cache;
  }
}
