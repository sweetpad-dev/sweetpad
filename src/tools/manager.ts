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
    const results = await Promise.all(
      TOOLS.map(async (item) => {
        try {
          await exec({
            command: item.check.command,
            args: item.check.args,
          });
          return {
            ...item,
            isInstalled: true,
          };
        } catch (error) {
          return {
            ...item,
            isInstalled: false,
          };
        }
      }),
    );
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
