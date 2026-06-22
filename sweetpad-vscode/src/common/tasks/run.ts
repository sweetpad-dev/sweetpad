import { getWorkspaceConfig } from "../config";
import type { ExecutionScopeService } from "../execution-scope";
import type { TaskExecutor, TaskTerminal } from "./types";
import { runTaskV2 } from "./v2";
import { runTaskV3 } from "./v3";

export function getTaskExecutorName(): TaskExecutor {
  const configured = getWorkspaceConfig("system.taskExecutor");
  if (configured === "v2") {
    return "v2";
  }
  return "v3";
}

export async function runTask<TMetadata>(
  execution: ExecutionScopeService,
  options: {
    name: string;
    source?: string;
    error?: string;
    problemMatchers?: string[];
    lock: string;
    metadata?: TMetadata;
    terminateLocked: boolean;
    callback: (terminal: TaskTerminal) => Promise<void>;
  },
): Promise<void> {
  const name = getTaskExecutorName();
  switch (name) {
    case "v2":
      return await runTaskV2(execution, options);
    case "v3":
      return await runTaskV3(execution, options);
    default:
      throw new Error(`Unknown executor: ${name}`);
  }
}
