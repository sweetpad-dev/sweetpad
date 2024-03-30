export class ExtensionError extends Error {
  context?: Record<string, unknown>;

  constructor(message: string, context?: Record<string, unknown>) {
    super(message);
    this.context = context;
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
    }
  ) {
    super(message, context);
  }
}

/**
 * Unkonwn error of executing shell command. See: exec
 */
export class ExecBaseError extends ExtensionError {
  constructor(
    message: string,
    options: { errorMessage: string; stderr?: string; command: string; args: string[]; cwd?: string }
  ) {
    super(message, options);
  }
}

/**
 * Stderr of executing shell command. See: exec
 */
export class ExecErrror extends ExecBaseError {
  constructor(
    message: string,
    options: { command: string; args: string[]; cwd?: string; exitCode: number; stderr: string; errorMessage: string }
  ) {
    super(message, options);
  }
}
