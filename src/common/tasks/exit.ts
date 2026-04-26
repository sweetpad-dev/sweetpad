import { ExecuteTaskError, type ProcessExit } from "./types";

/** Throws `ExecuteTaskError` on SIGINT (130) or non-zero exit code. */
export function assertCleanExit({ code, signal }: ProcessExit, command: string): void {
  if (signal === "SIGINT") {
    throw new ExecuteTaskError("Command was cancelled by user", { command, errorCode: 130 });
  }
  if (code !== 0) {
    throw new ExecuteTaskError("Command returned non-zero exit code", { command, errorCode: code });
  }
}
