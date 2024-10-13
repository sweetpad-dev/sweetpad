import { type XcodeScheme, getSchemes } from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { getCurrentXcodeWorkspacePath } from "./utils";

import events from "node:events";

type IEventMap = {
  updated: [];
  defaultSchemeUpdated: [scheme: string | undefined];
};
type IEventKey = keyof IEventMap;

export class BuildManager {
  private cache: XcodeScheme[] | undefined = undefined;
  private emitter = new events.EventEmitter<IEventMap>();
  public _context: ExtensionContext | undefined = undefined;

  on<K extends IEventKey>(event: K, listener: (...args: IEventMap[K]) => void): void {
    this.emitter.on(event, listener as any); // todo: fix this any
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
    this.emitter.emit("updated");
    return this.cache;
  }

  async getSchemas(options?: { refresh?: boolean }): Promise<XcodeScheme[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh();
    }
    return this.cache;
  }

  getDefaultScheme(): string | undefined {
    return this.context.getWorkspaceState("build.xcodeScheme");
  }

  setDefaultScheme(scheme: string | undefined): void {
    this.context.updateWorkspaceState("build.xcodeScheme", scheme);
    this.emitter.emit("defaultSchemeUpdated", scheme);
  }
}
