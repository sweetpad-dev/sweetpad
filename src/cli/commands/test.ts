import { isSuccess } from "../../protocol/envelope";
import { ProtocolError } from "../../protocol/errors";
import type { TestRequestParams, TestResponseData } from "../../protocol/types";
import { type ParsedArgs, getString } from "../argv";
import { exitCodeForErrorCode } from "../exit-codes";
import { type CommandEnv, resolveSocketPath, withClient } from "../runner";

export type TestCommandResult = {
  exitCode: number;
  envelope: object;
};

export type TestCommandEnv = CommandEnv;

/**
 * `sweetpad test` — runs `xcodebuild test` for the scheme and returns the
 * parsed .xcresult summary (counts + per-test outcomes). Exit code mirrors
 * the build status: 0 when all tests pass, 1 when any fail.
 */
export async function runTestCommand(args: ParsedArgs, env: TestCommandEnv): Promise<TestCommandResult> {
  const scheme = getString(args, "scheme");
  const destination = getString(args, "destination");
  const configuration = getString(args, "config") ?? getString(args, "configuration");
  const xcworkspaceOverride = getString(args, "xcworkspace");

  if (!scheme || !destination || !configuration) {
    throw new ProtocolError(
      "INVALID_ARGUMENT",
      "missing required flag(s): --scheme, --destination, --config",
      { hint: "sweetpad test --scheme=<name> --destination=<id-or-name> --config=<name>" },
    );
  }

  const socketPath = await resolveSocketPath(args, env);
  return await withClient(socketPath, async (client) => {
    const params: TestRequestParams = {
      scheme,
      destination,
      configuration,
      xcworkspace: xcworkspaceOverride,
    };
    const response = await client.request("test", params);

    if (!isSuccess(response)) {
      return { exitCode: exitCodeForErrorCode(response.error.code), envelope: response };
    }
    return { exitCode: exitCodeForTestStatus(response.data), envelope: response };
  });
}

function exitCodeForTestStatus(data: TestResponseData): number {
  // Even if the build "succeeded", any failing test should surface as
  // exit 1 so CI pipelines treat the run as failed. Use both
  // `testsFailed` and the underlying build status.
  if (data.testsFailed > 0) return 1;
  switch (data.status) {
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
