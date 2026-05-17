import { isSuccess } from "../../protocol/envelope";
import { ProtocolError } from "../../protocol/errors";
import type { LogsGetRequestParams } from "../../protocol/types";
import { type ParsedArgs, getString } from "../argv";
import { exitCodeForErrorCode } from "../exit-codes";
import { type CommandEnv, resolveSocketPath, withClient } from "../runner";

export type LogsCommandResult = {
  exitCode: number;
  envelope: object;
};

export type LogsCommandEnv = CommandEnv;

/**
 * `sweetpad logs <buildId> [--tail=<n>]` — print the captured xcodebuild log
 * for a build. The buildId is positional. With `--tail=<n>`, returns only
 * the last n lines (the envelope's `truncated` flag flips true when this
 * caps the output).
 */
export async function runLogsCommand(args: ParsedArgs, env: LogsCommandEnv): Promise<LogsCommandResult> {
  const buildId = args._[0];
  if (!buildId) {
    throw new ProtocolError("INVALID_ARGUMENT", "missing positional argument: <buildId>", {
      hint: "sweetpad logs <buildId> [--tail=<n>]",
    });
  }

  const tailStr = getString(args, "tail");
  let tail: number | undefined;
  if (tailStr !== undefined) {
    const parsed = Number.parseInt(tailStr, 10);
    if (!Number.isInteger(parsed) || parsed < 0) {
      throw new ProtocolError("INVALID_ARGUMENT", `--tail must be a non-negative integer (got '${tailStr}')`);
    }
    tail = parsed;
  }

  const socketPath = await resolveSocketPath(args, env);
  return await withClient(socketPath, async (client) => {
    const params: LogsGetRequestParams = { buildId, tail };
    const response = await client.request("logs.get", params);
    if (!isSuccess(response)) {
      return { exitCode: exitCodeForErrorCode(response.error.code), envelope: response };
    }
    return { exitCode: 0, envelope: response };
  });
}
