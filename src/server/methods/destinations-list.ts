import type { DestinationsManager } from "../../core/destination/manager";
import type { Destination, DestinationType } from "../../core/destination/types";
import { ProtocolError } from "../../protocol/errors";
import {
  ALL_DESTINATION_KINDS,
  type DestinationKind,
  type DestinationSummary,
  type DestinationsListRequestParams,
  type DestinationsListResponseData,
} from "../../protocol/types";

export type DestinationsListMethodDeps = {
  destinationsManager: DestinationsManager;
};

export function createDestinationsListMethod(deps: DestinationsListMethodDeps) {
  return async (rawParams: unknown): Promise<DestinationsListResponseData> => {
    const params = validateParams(rawParams);

    if (params.refresh) {
      await deps.destinationsManager.refresh();
    }

    let destinations = await deps.destinationsManager.getDestinations({ mostUsedSort: true });
    if (params.kind) {
      const kind = params.kind;
      destinations = destinations.filter((d) => (d.type as DestinationType) === kind);
    }

    return {
      destinations: destinations.map(toSummary),
    };
  };
}

function toSummary(d: Destination): DestinationSummary {
  return {
    id: d.id,
    // The engine's `DestinationType` is structurally identical to the wire's
    // `DestinationKind`. Re-asserting here keeps the protocol module free of
    // engine imports while still surfacing a runtime check if a new variant
    // is added on either side without updating the other.
    kind: assertDestinationKind(d.type),
    label: d.label,
    platform: d.platform,
  };
}

function assertDestinationKind(type: DestinationType): DestinationKind {
  if (!ALL_DESTINATION_KINDS.includes(type as DestinationKind)) {
    // This is a developer error — the engine knows about a destination type
    // the protocol doesn't. Surface it as INTERNAL rather than letting it
    // silently flow through as a stringly-typed mismatch.
    throw new ProtocolError("INTERNAL", `Unknown destination kind from engine: ${type}`);
  }
  return type as DestinationKind;
}

function validateParams(raw: unknown): DestinationsListRequestParams {
  if (raw === undefined || raw === null) return {};
  if (typeof raw !== "object") {
    throw new ProtocolError("INVALID_ARGUMENT", "destinations.list params must be an object");
  }
  const params = raw as Partial<DestinationsListRequestParams>;

  if (params.kind !== undefined) {
    if (typeof params.kind !== "string" || !ALL_DESTINATION_KINDS.includes(params.kind as DestinationKind)) {
      throw new ProtocolError(
        "INVALID_ARGUMENT",
        `'kind' must be one of: ${ALL_DESTINATION_KINDS.join(", ")}`,
      );
    }
  }
  if (params.refresh !== undefined && typeof params.refresh !== "boolean") {
    throw new ProtocolError("INVALID_ARGUMENT", "'refresh' must be a boolean");
  }

  return {
    kind: params.kind,
    refresh: params.refresh,
  };
}
