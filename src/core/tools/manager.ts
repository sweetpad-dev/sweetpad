import events from "node:events";

import { exec } from "../exec";
import type { Logger } from "../logger/types";
import type { WorkspaceRoot } from "../workspace-root";
import { TOOLS, type Tool } from "./constants";

type IEventMap = {
  updated: [];
};

type ToolItem = {
  isInstalled: boolean;
} & Tool;

export class ToolsManager {
  private cache: ToolItem[] | undefined = undefined;
  private logger: Logger;
  private workspaceRoot: WorkspaceRoot;

  private emitter = new events.EventEmitter<IEventMap>();

  constructor(options: { logger: Logger; workspaceRoot: WorkspaceRoot }) {
    this.logger = options.logger;
    this.workspaceRoot = options.workspaceRoot;
  }

  on(event: "updated", listener: () => void): void {
    this.emitter.on(event, listener);
  }

  async refresh(): Promise<ToolItem[]> {
    const cwd = this.workspaceRoot.getPath();
    const checks = await Promise.all(
      TOOLS.map(async (item) => {
        try {
          await exec({
            command: item.check.command,
            args: item.check.args,
            cwd: cwd,
            logger: this.logger,
          });
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
