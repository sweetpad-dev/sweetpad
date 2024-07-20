import { ExtensionContext } from "../common/commands";
import { iOSDevice, listDevices } from "../common/xcode/devicectl";
import events from "events";

type IEventMap = {
  refresh: [];
};

export class DevicesManager {
  private cache: iOSDevice[] | undefined = undefined;
  private _context: ExtensionContext | undefined = undefined;
  private emitter = new events.EventEmitter<IEventMap>();

  public failed: "unknown" | "no-devicectl" | null = null;

  set context(context: ExtensionContext) {
    this._context = context;
  }

  on(event: "refresh", listener: () => void): void {
    this.emitter.on(event, listener);
  }

  get context(): ExtensionContext {
    if (!this._context) {
      throw new Error("Context is not set");
    }
    return this._context;
  }

  async refresh(): Promise<iOSDevice[]> {
    this.failed = null;
    try {
      this.cache = await listDevices(this.context);
    } catch (error: any) {
      if (error?.error?.code === "ENOENT") {
        this.failed = "no-devicectl";
      } else {
        this.failed = "unknown";
      }
      this.cache = [];
    }
    this.emitter.emit("refresh");
    return this.cache;
  }

  async getDevices(options?: { refresh?: boolean }): Promise<iOSDevice[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh();
    }
    return this.cache;
  }
}
