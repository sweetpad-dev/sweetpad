import { isSuccess } from "../../protocol/envelope";
import type { SchemesListRequestParams } from "../../protocol/types";
import { type ParsedArgs, getString } from "../argv";
import { exitCodeForErrorCode } from "../exit-codes";
import { type CommandEnv, resolveSocketPath, withClient } from "../runner";

export type SchemesCommandResult = {
  exitCode: number;
  envelope: object;
};

export type SchemesCommandEnv = CommandEnv;

/**
 * `sweetpad schemes` — lists schemes defined in the workspace's xcworkspace.
 */
export async function runSchemesCommand(args: ParsedArgs, env: SchemesCommandEnv): Promise<SchemesCommandResult> {
  const xcworkspaceOverride = getString(args, "xcworkspace");

  const socketPath = await resolveSocketPath(args, env);
  return await withClient(socketPath, async (client) => {
    const params: SchemesListRequestParams = { xcworkspace: xcworkspaceOverride };
    const response = await client.request("schemes.list", params);

    if (!isSuccess(response)) {
      return { exitCode: exitCodeForErrorCode(response.error.code), envelope: response };
    }
    return { exitCode: 0, envelope: response };
  });
}
