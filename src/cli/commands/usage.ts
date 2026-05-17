import { isSuccess } from "../../protocol/envelope";
import type { ParsedArgs } from "../argv";
import { exitCodeForErrorCode } from "../exit-codes";
import { type CommandEnv, resolveSocketPath, withClient } from "../runner";

export type UsageCommandResult = {
  exitCode: number;
  envelope: object;
};

export type UsageCommandEnv = CommandEnv;

/**
 * `sweetpad usage` — enumerates every method the server exposes. Agents call
 * this to discover what's available without baking the method list into
 * their prompt.
 */
export async function runUsageCommand(args: ParsedArgs, env: UsageCommandEnv): Promise<UsageCommandResult> {
  const socketPath = await resolveSocketPath(args, env);
  return await withClient(socketPath, async (client) => {
    const response = await client.request("usage", {});
    if (!isSuccess(response)) {
      return { exitCode: exitCodeForErrorCode(response.error.code), envelope: response };
    }
    return { exitCode: 0, envelope: response };
  });
}
