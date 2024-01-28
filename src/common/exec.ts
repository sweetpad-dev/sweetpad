import { Timer } from "./timer.js";

export type ExecaError = {
  command: string;
  escapedCommand: string;
  exitCode: number;
  stdout: string;
  stderr: string;
  failed: boolean;
  timedOut: boolean;
  killed: boolean;
  signal?: string;
  signalDescription?: string;
  cwd: string;
  message: string;
  shortMessage: string;
  originalMessage: string;
};

type ExecResult = {
  stdout: string;
  error?: ExecaError;
  time: number;
};

/**
 * Execa is ESM only, so we need to import it dynamically, because we are using CJS and
 * can't move to ESM yet due to vscode limitations
 */
async function getExeca() {
  return await import("execa");
}

export async function preloadExec() {
  await getExeca();
}

/**
 * Tagged template literal that executes a command and returns the result object. Should never throw an error.
 */
export async function exec(command: TemplateStringsArray, ...values: any[]): Promise<ExecResult> {
  const timer = new Timer();
  const execa = await getExeca();
  try {
    const output = await execa.$(command, ...values);
    return {
      stdout: output.stdout,
      time: timer.elapsed,
    };
  } catch (e: any) {
    return {
      stdout: "",
      error: e,
      time: timer.elapsed,
    };
  }
}

/**
 * Execute already prepared command and return the result object. Should never throw an error.
 */
export async function execPrepared(command: string): Promise<ExecResult> {
  const timer = new Timer();
  const execa = await getExeca();
  try {
    const output = await execa.execaCommand(command);
    return {
      stdout: output.stdout,
      time: timer.elapsed,
    };
  } catch (e: any) {
    return {
      stdout: "",
      error: e,
      time: timer.elapsed,
    };
  }
}
