import type { Destination } from "../../destination/types";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES, type DestinationEntity } from "../types";
import type { HandlerFn } from "./context";

const SIMULATOR_TYPES = new Set(["iOSSimulator", "watchOSSimulator", "tvOSSimulator", "visionOSSimulator"]);

function toEntity(d: Destination, selectedId: string | undefined): DestinationEntity {
  const simulatorState = SIMULATOR_TYPES.has(d.type)
    ? ((d as { state?: "Booted" | "Shutdown" }).state ?? undefined)
    : undefined;
  return {
    id: d.id,
    name: d.name,
    type: d.type,
    platform: d.platform,
    isSelected: selectedId === d.id,
    simulatorState,
  };
}

export const destinationList: HandlerFn<
  { type?: string; platform?: string; booted?: boolean },
  { destinations: DestinationEntity[] }
> = async (params, ctx) => {
  const destinations = await ctx.destinationsManager.getDestinations({ mostUsedSort: true });
  const selected = ctx.destinationsManager.getSelectedXcodeDestinationForBuild();
  const entities = destinations.map((d) => toEntity(d, selected?.id));
  const typeFilter = params?.type;
  const platformFilter = params?.platform;
  const bootedFilter = params?.booted;
  const filtered = entities.filter((e) => {
    if (typeFilter && e.type !== typeFilter) return false;
    if (platformFilter && e.platform !== platformFilter) return false;
    if (bootedFilter !== undefined && bootedFilter !== (e.simulatorState === "Booted")) return false;
    return true;
  });
  return { destinations: filtered };
};

export const destinationGet: HandlerFn<unknown, { destination: DestinationEntity | null }> = async (_params, ctx) => {
  const selected = ctx.destinationsManager.getSelectedXcodeDestinationForBuild();
  if (!selected) return { destination: null };
  const all = await ctx.destinationsManager.getDestinations();
  const match = all.find((d) => d.id === selected.id);
  if (!match) {
    // Synthesize when the persisted selection isn't in the current scan.
    return {
      destination: {
        id: selected.id,
        name: selected.name,
        type: selected.type,
        isSelected: true,
      } satisfies DestinationEntity,
    };
  }
  return { destination: toEntity(match, selected.id) };
};

export const destinationSet: HandlerFn<{ id?: string }, { destination: DestinationEntity }> = async (params, ctx) => {
  if (!params?.id || typeof params.id !== "string") {
    throw new SweetpadRpcError(ERROR_CODES.INVALID_PARAMS, "destination.set requires { id: string }");
  }
  const all = await ctx.destinationsManager.getDestinations();
  const match = all.find((d) => d.id === params.id);
  if (!match) {
    throw new SweetpadRpcError(ERROR_CODES.DESTINATION_NOT_FOUND, `Destination not found: ${params.id}`, {
      hint: "sweetpad vscode destination.list",
    });
  }
  ctx.destinationsManager.setWorkspaceDestinationForBuild(match);
  return { destination: toEntity(match, match.id) };
};
