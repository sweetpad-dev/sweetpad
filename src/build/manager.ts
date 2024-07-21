import { getSchemes, XcodeScheme } from "../common/cli/scripts";
import { ExtensionContext } from "../common/commands";
import { getCurrentXcodeWorkspacePath } from "./utils";

import events from "events";

type IEventMap = {
  refresh: [];
};

export class BuildManager {
  private cache: XcodeScheme[] | undefined = undefined;
  private emitter = new events.EventEmitter<IEventMap>();
  public _context: ExtensionContext | undefined = undefined;

  on(event: "refresh", listener: () => void): void {
    this.emitter.on(event, listener);
  }

  set context(context: ExtensionContext) {
    this._context = context;
  }

  get context(): ExtensionContext {
    if (!this._context) {
      throw new Error("Context is not set");
    }
    return this._context;
  }

  async refresh(): Promise<XcodeScheme[]> {
    const xcworkspace = getCurrentXcodeWorkspacePath(this.context);

    const scheme = await getSchemes({
      xcworkspace: xcworkspace,
    });
    this.cache = scheme;
    this.emitter.emit("refresh");
    return this.cache;
  }

  async getSchemas(options?: { refresh?: boolean }): Promise<XcodeScheme[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh();
    }
    return this.cache;
  }
}
