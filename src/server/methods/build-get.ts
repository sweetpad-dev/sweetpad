import { ProtocolError } from "../../protocol/errors";
import type { BuildGetRequestParams, BuildGetResponseData } from "../../protocol/types";
import type { BuildRegistry } from "../registry";

export type BuildGetMethodDeps = {
  registry: BuildRegistry;
};

export function createBuildGetMethod(deps: BuildGetMethodDeps) {
  return async (rawParams: unknown): Promise<BuildGetResponseData> => {
    const params = validateParams(rawParams);

    const build = deps.registry.get(params.buildId);
    if (!build) {
      throw new ProtocolError("BUILD_NOT_FOUND", `No build with id '${params.buildId}'`, {
        hint: "sweetpad builds — list everything in the registry",
      });
    }
    return build;
  };
}

function validateParams(raw: unknown): BuildGetRequestParams {
  if (!raw || typeof raw !== "object") {
    throw new ProtocolError("INVALID_ARGUMENT", "Missing build.get params");
  }
  const params = raw as Partial<BuildGetRequestParams>;
  if (typeof params.buildId !== "string" || params.buildId.length === 0) {
    throw new ProtocolError("INVALID_ARGUMENT", "'buildId' is required and must be a non-empty string");
  }
  return { buildId: params.buildId };
}
