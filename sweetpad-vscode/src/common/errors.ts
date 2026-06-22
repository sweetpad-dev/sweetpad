import { commonLogger } from "./logger";

export type ErrorMessageAction = {
  label: string;
  callback: () => void;
};

type ExtensionErrorOptions = {
  actions?: ErrorMessageAction[];
  context?: Record<string, unknown>;
};

/**
 * Basic generic error for extension. Throw this error if you don't know what to throw.
 */
export class ExtensionError extends Error {
  options?: ExtensionErrorOptions;

  constructor(message: string, options?: ExtensionErrorOptions) {
    super(message);
    commonLogger.debug("ExecBaseError constructor", {
      errorMessage: message,
      errorOptions: options,
    });
    this.options = options;
  }
}

/**
 * Error of executing shell task. See: runShellTask
 */
export class TaskError extends ExtensionError {
  constructor(
    message: string,
    context: {
      name: string;
      soruce?: string;
      command?: string;
      args?: string[];
      errorCode?: number;
    },
  ) {
    super(message, { context });
  }
}

/**
 * Unkonwn error of executing shell command. See: exec
 */
export class ExecBaseError extends ExtensionError {
  constructor(
    message: string,
    context: { errorMessage: string; stderr?: string; command: string; args: string[]; cwd?: string },
  ) {
    super(message, { context });
  }
}

/**
 * Stderr of executing shell command. See: exec
 */
export class ExecError extends ExecBaseError {}
