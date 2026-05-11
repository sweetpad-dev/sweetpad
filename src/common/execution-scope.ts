import { AsyncLocalStorage } from "node:async_hooks";
import * as crypto from "node:crypto";
import * as events from "node:events";

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

type EventMap = {
  closed: [scope: ExecutionScope];
};

export class ExecutionScopeService {
  private storage = new AsyncLocalStorage<ExecutionScope | undefined>();
  private emitter = new events.EventEmitter<EventMap>();

  /**
   * Run a callback inside a pre-existing scope without firing `closed`.
   * Use this to propagate the current scope into a detached async context.
   */
  setScope<T>(scope: ExecutionScope | undefined, callback: () => Promise<T>): Promise<T> {
    return this.storage.run(scope, callback);
  }

  /**
   * Start a new scope and fire `closed` when the callback settles.
   */
  startScope<T>(scope: ExecutionScope, callback: () => Promise<T>): Promise<T> {
    return this.storage.run(scope, async () => {
      try {
        return await callback();
      } finally {
        this.emitter.emit("closed", scope);
      }
    });
  }

  getScope(): ExecutionScope | undefined {
    return this.storage.getStore();
  }

  getScopeId(): string | undefined {
    return this.getScope()?.id;
  }

  onClosed(listener: (scope: ExecutionScope) => void): void {
    this.emitter.on("closed", listener);
  }
}
