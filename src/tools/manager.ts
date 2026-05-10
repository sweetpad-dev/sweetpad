import events from "node:events";

import { exec } from "../common/exec";
import { TOOLS, type Tool } from "./constants";

type IEventMap = {
  updated: [];
};

type ToolItem = {
  isInstalled: boolean;
} & Tool;

export class ToolsManager {
  private cache: ToolItem[] | undefined = undefined;

  private emitter = new events.EventEmitter<IEventMap>();

  on(event: "updated", listener: () => void): void {
    this.emitter.on(event, listener);
  }

  async refresh(): Promise<ToolItem[]> {
    const checks = await Promise.all(
      TOOLS.map(async (item) => {
        try {
          await exec({ command: item.check.command, args: item.check.args });
          return true;
        } catch {
          return false;
        }
      }),
    );
    const results: ToolItem[] = TOOLS.map((item, i) => ({
      id: item.id,
      label: item.label,
      check: item.check,
      install: item.install,
      documentation: item.documentation,
      isInstalled: checks[i],
    }));
    this.cache = results;
    this.emitter.emit("updated");
    return this.cache;
  }

  async getTools(options?: { refresh?: boolean }): Promise<ToolItem[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh();
    }
    return this.cache;
  }
}
