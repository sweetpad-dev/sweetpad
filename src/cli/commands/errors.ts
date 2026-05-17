import { errorResponse, isSuccess } from "../../protocol/envelope";
import type {
  BuildGetRequestParams,
  BuildResponseData,
  BuildsListRequestParams,
  BuildsListResponseData,
  WireResponse,
} from "../../protocol/types";
import { type ParsedArgs, getString } from "../argv";
import { exitCodeForErrorCode } from "../exit-codes";
import type { ProtocolClient } from "../protocol";
import { type CommandEnv, resolveSocketPath, withClient } from "../runner";

export type ErrorsCommandResult = {
  exitCode: number;
  envelope: object;
};

export type ErrorsCommandEnv = CommandEnv;

/**
 * `sweetpad errors [--build=<id>]` — surface a build's diagnostics. Without
 * `--build`, picks the most recent failed build. CLI-side sugar over
 * `builds.list` + `build.get` — no dedicated wire method.
 */
export async function runErrorsCommand(args: ParsedArgs, env: ErrorsCommandEnv): Promise<ErrorsCommandResult> {
  const buildIdOverride = getString(args, "build");

  const socketPath = await resolveSocketPath(args, env);
  return await withClient(socketPath, async (client) => {
    if (buildIdOverride) {
      return await fetchByBuildId(client, buildIdOverride);
    }
    return await fetchMostRecentFailed(client);
  });
}

async function fetchByBuildId(client: ProtocolClient, buildId: string): Promise<ErrorsCommandResult> {
  const params: BuildGetRequestParams = { buildId };
  const response = await client.request("build.get", params);
  if (!isSuccess(response)) {
    return { exitCode: exitCodeForErrorCode(response.error.code), envelope: response };
  }
  return { exitCode: 0, envelope: response };
}

async function fetchMostRecentFailed(client: ProtocolClient): Promise<ErrorsCommandResult> {
  const listParams: BuildsListRequestParams = { status: "failed", limit: 1 };
  const listResponse = (await client.request("builds.list", listParams)) as WireResponse<BuildsListResponseData>;
  if (!isSuccess(listResponse)) {
    return { exitCode: exitCodeForErrorCode(listResponse.error.code), envelope: listResponse };
  }

  const [latest] = listResponse.data.builds;
  if (!latest) {
    const envelope = errorResponse(
      0,
      {
        code: "BUILD_NOT_FOUND",
        message: "No failed builds in this workspace's history",
        hint: "sweetpad builds — list everything",
      },
      undefined,
    );
    return { exitCode: exitCodeForErrorCode("BUILD_NOT_FOUND"), envelope };
  }
  return wrapBuild(latest);
}

// `builds.list` already returns the full BuildResponseData, so we skip the
// follow-up `build.get`. Wrap the snapshot in the same envelope shape `show`
// produces so all the read commands look identical on stdout.
function wrapBuild(build: BuildResponseData): ErrorsCommandResult {
  return {
    exitCode: 0,
    envelope: {
      id: 0,
      ok: true as const,
      schemaVersion: "1.0",
      data: build,
    },
  };
}
