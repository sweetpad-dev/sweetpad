import { AsyncLocalStorage } from "node:async_hooks";
import * as crypto from "node:crypto";
import type { ExtensionContext } from "./context";

/**
 * Execution scope manager allows to create isolated execution contexts
 * for commands, tasks, and other operations that require a specific context.
 *
 * It uses AsyncLocalStorage to propagate the execution context
 * across asynchronous operations.
 */
export class ExecutionScopeManager {
  context: ExtensionContext;
  private executionScope = new AsyncLocalStorage<ExecutionScope | undefined>();

  constructor(options: {
    context: ExtensionContext;
  }) {
    this.context = options.context;
  }

  /**
   * In case if you need to start propage execution scope manually you can use this method
   */
  setCurrent<T>(scope: ExecutionScope | undefined, callback: () => Promise<T>): Promise<T> {
    return this.executionScope.run(scope, callback);
  }

  getCurrent(): ExecutionScope | undefined {
    return this.executionScope.getStore();
  }

  getCurrentId(): string | undefined {
    return this.getCurrent()?.id;
  }

  /**
   * Main method to start execution scope for command or task or other isolated execution context
   */
  start<T>(scope: ExecutionScope, callback: () => Promise<T>): Promise<T> {
    return this.executionScope.run(scope, async () => {
      try {
        return await callback();
      } finally {
        this.context.emitter.emit("executionScopeClosed", scope);
      }
    });
  }
}

export class BaseExecutionScope {
  id: string;
  type = "base" as const;

  constructor() {
    this.id = crypto.randomUUID();
    this.type = "base";
  }
}

export class CommandExecutionScope {
  id: string;
  type = "command" as const;
  commandName: string;

  constructor(options: { commandName: string }) {
    this.id = crypto.randomUUID();
    this.type = "command";
    this.commandName = options.commandName;
  }
}

export class TaskExecutionScope {
  id: string;
  type = "task" as const;
  taskName: string;

  constructor(options: { action: string }) {
    this.id = crypto.randomUUID();
    this.type = "task";
    this.taskName = options.action;
  }
}

export type ExecutionScope = BaseExecutionScope | CommandExecutionScope | TaskExecutionScope;
