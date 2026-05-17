import { isSuccess } from "../../protocol/envelope";
import { ProtocolError } from "../../protocol/errors";
import {
  ALL_DESTINATION_KINDS,
  type DestinationKind,
  type DestinationsListRequestParams,
} from "../../protocol/types";
import { type ParsedArgs, getBool, getString } from "../argv";
import { exitCodeForErrorCode } from "../exit-codes";
import { type CommandEnv, resolveSocketPath, withClient } from "../runner";

export type DestinationsCommandResult = {
  exitCode: number;
  envelope: object;
};

export type DestinationsCommandEnv = CommandEnv;

/**
 * `sweetpad destinations` — lists simulators / devices the build command can
 * target. `--kind=<DestinationKind>` filters server-side. `--refresh` forces
 * the engine's simctl / xctrace caches to repopulate before listing.
 */
export async function runDestinationsCommand(
  args: ParsedArgs,
  env: DestinationsCommandEnv,
): Promise<DestinationsCommandResult> {
  const kindArg = getString(args, "kind");
  const refresh = getBool(args, "refresh");

  if (kindArg !== undefined && !ALL_DESTINATION_KINDS.includes(kindArg as DestinationKind)) {
    throw new ProtocolError(
      "INVALID_ARGUMENT",
      `invalid --kind value '${kindArg}'`,
      { hint: `one of: ${ALL_DESTINATION_KINDS.join(", ")}` },
    );
  }

  const socketPath = await resolveSocketPath(args, env);
  return await withClient(socketPath, async (client) => {
    const params: DestinationsListRequestParams = {
      kind: kindArg as DestinationKind | undefined,
      refresh,
    };
    const response = await client.request("destinations.list", params);

    if (!isSuccess(response)) {
      return { exitCode: exitCodeForErrorCode(response.error.code), envelope: response };
    }
    return { exitCode: 0, envelope: response };
  });
}
