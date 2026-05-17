import { isSuccess } from "../../protocol/envelope";
import { ProtocolError } from "../../protocol/errors";
import type { RunRequestParams, RunResponseData } from "../../protocol/types";
import { type ParsedArgs, getBool, getString } from "../argv";
import { exitCodeForErrorCode } from "../exit-codes";
import { type CommandEnv, resolveSocketPath, withClient } from "../runner";

export type RunCommandResult = {
  exitCode: number;
  envelope: object;
};

export type RunCommandEnv = CommandEnv;

/**
 * `sweetpad run` — builds, installs, and launches the app on the selected
 * destination. Blocks until the launched app exits.
 *
 * Launch args/env aren't a CLI flag in v1 — the scheme's `<LaunchAction>`
 * args/env still apply, as do `sweetpad.build.launchArgs` /
 * `sweetpad.build.launchEnv` from workspace config. The wire method itself
 * does accept them, ready for a follow-up that exposes them on the CLI.
 */
export async function runRunCommand(args: ParsedArgs, env: RunCommandEnv): Promise<RunCommandResult> {
  const scheme = getString(args, "scheme");
  const destination = getString(args, "destination");
  const configuration = getString(args, "config") ?? getString(args, "configuration");
  const xcworkspaceOverride = getString(args, "xcworkspace");
  const debug = getBool(args, "debug");

  if (!scheme || !destination || !configuration) {
    throw new ProtocolError(
      "INVALID_ARGUMENT",
      "missing required flag(s): --scheme, --destination, --config",
      { hint: "sweetpad run --scheme=<name> --destination=<id-or-name> --config=<name>" },
    );
  }

  const socketPath = await resolveSocketPath(args, env);
  return await withClient(socketPath, async (client) => {
    const params: RunRequestParams = {
      scheme,
      destination,
      configuration,
      xcworkspace: xcworkspaceOverride,
      debug,
    };
    const response = await client.request("run", params);

    if (!isSuccess(response)) {
      return { exitCode: exitCodeForErrorCode(response.error.code), envelope: response };
    }
    return { exitCode: exitCodeForBuildStatus(response.data), envelope: response };
  });
}

function exitCodeForBuildStatus(build: RunResponseData): number {
  switch (build.status) {
    case "succeeded":
      return 0;
    case "failed":
    case "cancelled":
    case "interrupted":
      return 1;
    case "running":
      return 0;
  }
}
