import { isSuccess } from "../../protocol/envelope";
import { ProtocolError } from "../../protocol/errors";
import type { BuildRequestParams, BuildResponseData } from "../../protocol/types";
import { type ParsedArgs, getBool, getString } from "../argv";
import { exitCodeForErrorCode } from "../exit-codes";
import { type CommandEnv, resolveSocketPath, withClient } from "../runner";
//
// `BuildRequestParams` / `BuildResponseData` here aren't re-exports — they're
// the same shapes the server's `methods/build.ts` consumes via the shared
// `MethodMap`. Drifting any field is a compile error in both binaries.

export type BuildCommandResult = {
  exitCode: number;
  envelope: object;
};

export type BuildCommandEnv = CommandEnv;

/**
 * Drives a `sweetpad build` invocation: validates flags, ensures the server
 * is up, sends the request, prints the envelope to stdout, and maps the
 * Build status / error code to an exit code per `docs/dev/agent-cli.md` §5.
 */
export async function runBuildCommand(args: ParsedArgs, env: BuildCommandEnv): Promise<BuildCommandResult> {
  const scheme = getString(args, "scheme");
  const destination = getString(args, "destination");
  const configuration = getString(args, "config") ?? getString(args, "configuration");
  // `--xcworkspace` points at a specific `.xcworkspace` / `Package.swift`
  // when there are multiple inside the root. `--workspace` (handled by the
  // shared runner) overrides the *root* (cwd-walk target + socket key).
  const xcworkspaceOverride = getString(args, "xcworkspace");
  const debug = getBool(args, "debug");

  if (!scheme || !destination || !configuration) {
    throw new ProtocolError(
      "INVALID_ARGUMENT",
      "missing required flag(s): --scheme, --destination, --config",
      { hint: "sweetpad build --scheme=<name> --destination=<id-or-name> --config=<name>" },
    );
  }

  const socketPath = await resolveSocketPath(args, env);
  return await withClient(socketPath, async (client) => {
    const params: BuildRequestParams = {
      scheme,
      destination,
      configuration,
      xcworkspace: xcworkspaceOverride,
      debug,
    };
    const response = await client.request("build", params);

    if (!isSuccess(response)) {
      return { exitCode: exitCodeForErrorCode(response.error.code), envelope: response };
    }

    const exitCode = exitCodeForBuildStatus(response.data);
    return { exitCode, envelope: response };
  });
}

function exitCodeForBuildStatus(build: BuildResponseData): number {
  switch (build.status) {
    case "succeeded":
      return 0;
    case "failed":
    case "cancelled":
    case "interrupted":
      return 1;
    case "running":
      // Shouldn't happen in v1 — the server blocks until the build settles
      // before responding. If it does (later iterations with --wait timeout),
      // exit 0 and let the caller poll.
      return 0;
  }
}
