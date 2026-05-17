import type { BuildManager } from "../../core/build/manager";
import type { ConfigProvider } from "../../core/config/types";
import type { WorkspaceState } from "../../core/state/types";
import type { WorkspaceRoot } from "../../core/workspace-root";
import { ProtocolError } from "../../protocol/errors";
import type { SchemesListRequestParams, SchemesListResponseData } from "../../protocol/types";
import { resolveXcworkspace } from "./helpers";

export type SchemesListMethodDeps = {
  buildManager: BuildManager;
  workspaceRoot: WorkspaceRoot;
  config: ConfigProvider;
  state: WorkspaceState;
};

export function createSchemesListMethod(deps: SchemesListMethodDeps) {
  return async (rawParams: unknown): Promise<SchemesListResponseData> => {
    const params = validateParams(rawParams);

    const xcworkspace = await resolveXcworkspace(deps, params.xcworkspace);
    // The build manager keys its cache off `state["build.xcodeWorkspacePath"]`,
    // so persist the resolved path before refreshing — otherwise a follow-up
    // `build` call against a different xcworkspace would surface stale data.
    deps.state.update("build.xcodeWorkspacePath", xcworkspace);

    const schemes = await deps.buildManager.getSchemes();
    return {
      schemes: schemes.map((s) => ({ name: s.name })),
      xcworkspace,
    };
  };
}

function validateParams(raw: unknown): SchemesListRequestParams {
  if (raw === undefined || raw === null) return {};
  if (typeof raw !== "object") {
    throw new ProtocolError("INVALID_ARGUMENT", "schemes.list params must be an object");
  }
  const params = raw as Partial<SchemesListRequestParams>;
  if (params.xcworkspace !== undefined && typeof params.xcworkspace !== "string") {
    throw new ProtocolError("INVALID_ARGUMENT", "'xcworkspace' must be a string");
  }
  return { xcworkspace: params.xcworkspace };
}
