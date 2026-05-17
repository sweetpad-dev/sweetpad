import * as vscode from "vscode";

import type { ConfigProvider } from "../../core/config/types";
import type { ExecutionScopeService } from "../../core/execution-scope";
import type { Logger } from "../../core/logger/types";
import type { TaskRunner, TaskTerminal } from "../../core/tasks/types";
import type { WorkspaceRoot } from "../../core/workspace-root";
import { runTaskV2 } from "./v2";
import { runTaskV3 } from "./v3";

export function getTaskExecutorName(config: ConfigProvider): "v2" | "v3" {
  const configured = config.get("system.taskExecutor");
  if (configured === "v2") {
    return "v2";
  }
  return "v3";
}

export type VsCodeTaskRunnerDeps = {
  execution: ExecutionScopeService;
  config: ConfigProvider;
  workspaceRoot: WorkspaceRoot;
  logger: Logger;
};

/**
 * VS Code-backed TaskRunner. Dispatches between `runTaskV2` (the plain
 * `vscode.tasks.executeTask` path) and `runTaskV3` (the node-pty path) based on
 * `sweetpad.system.taskExecutor`.
 */
export class VsCodeTaskRunner implements TaskRunner {
  constructor(private readonly deps: VsCodeTaskRunnerDeps) {}

  async run<TMetadata>(options: {
    name: string;
    source?: string;
    error?: string;
    problemMatchers?: string[];
    lock: string;
    metadata?: TMetadata;
    terminateLocked: boolean;
    callback: (terminal: TaskTerminal) => Promise<void>;
  }): Promise<void> {
    const executor = getTaskExecutorName(this.deps.config);
    switch (executor) {
      case "v2":
        return await runTaskV2(this.deps, options);
      case "v3":
        return await runTaskV3(this.deps, options);
      default:
        throw new Error(`Unknown executor: ${executor}`);
    }
  }

  stopMatching(predicate: { lock: string; metadata?: Record<string, unknown> }): void {
    const tasks = vscode.tasks.taskExecutions.filter(({ task }) => {
      if (task.definition.lockId !== predicate.lock) return false;
      if (predicate.metadata) {
        for (const [key, value] of Object.entries(predicate.metadata)) {
          if (task.definition.metadata?.[key] !== value) return false;
        }
      }
      return true;
    });
    for (const task of tasks) {
      task.terminate();
    }
  }
}
