import { isSuccess } from "../../protocol/envelope";
import { ProtocolError } from "../../protocol/errors";
import type { BuildGetRequestParams } from "../../protocol/types";
import type { ParsedArgs } from "../argv";
import { exitCodeForErrorCode } from "../exit-codes";
import { type CommandEnv, resolveSocketPath, withClient } from "../runner";

export type ShowCommandResult = {
  exitCode: number;
  envelope: object;
};

export type ShowCommandEnv = CommandEnv;

/**
 * `sweetpad show <buildId>` — print one build's full snapshot. The buildId
 * is positional; everything else uses the standard `CommandEnv` flags.
 */
export async function runShowCommand(args: ParsedArgs, env: ShowCommandEnv): Promise<ShowCommandResult> {
  const buildId = args._[0];
  if (!buildId) {
    throw new ProtocolError("INVALID_ARGUMENT", "missing positional argument: <buildId>", {
      hint: "sweetpad show <buildId> — e.g. `sweetpad show b1`",
    });
  }

  const socketPath = await resolveSocketPath(args, env);
  return await withClient(socketPath, async (client) => {
    const params: BuildGetRequestParams = { buildId };
    const response = await client.request("build.get", params);
    if (!isSuccess(response)) {
      return { exitCode: exitCodeForErrorCode(response.error.code), envelope: response };
    }
    return { exitCode: 0, envelope: response };
  });
}
