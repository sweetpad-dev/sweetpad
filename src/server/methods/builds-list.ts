import { ProtocolError } from "../../protocol/errors";
import type {
  BuildStatus,
  BuildsListRequestParams,
  BuildsListResponseData,
} from "../../protocol/types";
import type { BuildRegistry } from "../registry";

const ALL_STATUSES: BuildStatus[] = ["running", "succeeded", "failed", "cancelled", "interrupted"];

export type BuildsListMethodDeps = {
  registry: BuildRegistry;
};

export function createBuildsListMethod(deps: BuildsListMethodDeps) {
  return async (rawParams: unknown): Promise<BuildsListResponseData> => {
    const params = validateParams(rawParams);

    // The registry returns builds in allocation order (b1, b2, ...). Reverse
    // so callers see most-recent-first without paging logic on their side.
    let builds = deps.registry.list().slice().reverse();
    if (params.status) {
      const status = params.status;
      builds = builds.filter((b) => b.status === status);
    }
    if (params.limit !== undefined) {
      builds = builds.slice(0, params.limit);
    }

    return { builds };
  };
}

function validateParams(raw: unknown): BuildsListRequestParams {
  if (raw === undefined || raw === null) return {};
  if (typeof raw !== "object") {
    throw new ProtocolError("INVALID_ARGUMENT", "builds.list params must be an object");
  }
  const params = raw as Partial<BuildsListRequestParams>;

  if (params.limit !== undefined) {
    if (typeof params.limit !== "number" || !Number.isInteger(params.limit) || params.limit < 0) {
      throw new ProtocolError("INVALID_ARGUMENT", "'limit' must be a non-negative integer");
    }
  }
  if (params.status !== undefined) {
    if (typeof params.status !== "string" || !ALL_STATUSES.includes(params.status as BuildStatus)) {
      throw new ProtocolError(
        "INVALID_ARGUMENT",
        `'status' must be one of: ${ALL_STATUSES.join(", ")}`,
      );
    }
  }

  return { limit: params.limit, status: params.status };
}
