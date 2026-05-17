import { isSuccess } from "../../protocol/envelope";
import { ProtocolError } from "../../protocol/errors";
import type { BuildStatus, BuildsListRequestParams } from "../../protocol/types";
import { type ParsedArgs, getString } from "../argv";
import { exitCodeForErrorCode } from "../exit-codes";
import { type CommandEnv, resolveSocketPath, withClient } from "../runner";

const ALL_STATUSES: BuildStatus[] = ["running", "succeeded", "failed", "cancelled", "interrupted"];

export type BuildsCommandResult = {
  exitCode: number;
  envelope: object;
};

export type BuildsCommandEnv = CommandEnv;

/**
 * `sweetpad builds` — lists builds the server has on disk (most recent
 * first). Filter with `--status=<status>`; cap with `--limit=<n>`.
 */
export async function runBuildsCommand(args: ParsedArgs, env: BuildsCommandEnv): Promise<BuildsCommandResult> {
  const status = getString(args, "status") as BuildStatus | undefined;
  const limitStr = getString(args, "limit");
  let limit: number | undefined;
  if (limitStr !== undefined) {
    const parsed = Number.parseInt(limitStr, 10);
    if (!Number.isInteger(parsed) || parsed < 0) {
      throw new ProtocolError("INVALID_ARGUMENT", `--limit must be a non-negative integer (got '${limitStr}')`);
    }
    limit = parsed;
  }
  if (status !== undefined && !ALL_STATUSES.includes(status)) {
    throw new ProtocolError(
      "INVALID_ARGUMENT",
      `invalid --status value '${status}'`,
      { hint: `one of: ${ALL_STATUSES.join(", ")}` },
    );
  }

  const socketPath = await resolveSocketPath(args, env);
  return await withClient(socketPath, async (client) => {
    const params: BuildsListRequestParams = { status, limit };
    const response = await client.request("builds.list", params);
    if (!isSuccess(response)) {
      return { exitCode: exitCodeForErrorCode(response.error.code), envelope: response };
    }
    return { exitCode: 0, envelope: response };
  });
}
