import { execa } from "execa";

import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES } from "../types";
import { requireString } from "./_common";
import type { HandlerFn, RpcContext } from "./context";

export type SimulatorEntity = {
  id: string;
  udid: string;
  name: string;
  type: string;
  platform: string | undefined;
  state: "Booted" | "Shutdown";
  isAvailable: boolean;
};

export async function loadSimulators(ctx: RpcContext): Promise<SimulatorEntity[]> {
  const list = await ctx.destinationsManager.getSimulators();
  return list.map((s) => ({
    id: s.id,
    udid: s.udid,
    name: s.name,
    type: s.type,
    platform: s.platform,
    state: s.state,
    isAvailable: s.isAvailable,
  }));
}

export async function findSimulator(
  ctx: RpcContext,
  idOrUdid: string,
  options?: { requireBooted?: boolean },
): Promise<SimulatorEntity> {
  const all = await loadSimulators(ctx);
  const match = all.find((s) => s.udid === idOrUdid || s.id === idOrUdid);
  if (!match) {
    throw new SweetpadRpcError(ERROR_CODES.SIMULATOR_NOT_FOUND, `Simulator not found: ${idOrUdid}`, {
      hint: "sweetpad simulator.list",
    });
  }
  if (options?.requireBooted && match.state !== "Booted") {
    throw new SweetpadRpcError(ERROR_CODES.SIMCTL_FAILED, `Simulator "${match.name}" is not booted.`, {
      hint: `sweetpad simulator.start ${match.udid}`,
    });
  }
  return match;
}

export const simulatorList: HandlerFn<
  { state?: string; available?: boolean },
  { simulators: SimulatorEntity[] }
> = async (params, ctx) => {
  const simulators = await loadSimulators(ctx);
  const filtered = simulators.filter((s) => {
    if (params?.state && s.state !== params.state) return false;
    if (params?.available !== undefined && s.isAvailable !== params.available) return false;
    return true;
  });
  return { simulators: filtered };
};

export const simulatorStart: HandlerFn<
  { id?: string },
  { booted: true; alreadyRunning: boolean; simulator: SimulatorEntity }
> = async (params, ctx) => {
  const id = requireString(params?.id, "simulator.start", "id");
  const sim = await findSimulator(ctx, id);
  if (sim.state === "Booted") return { booted: true, alreadyRunning: true, simulator: sim };
  try {
    await execa("xcrun", ["simctl", "boot", sim.udid], { cwd: ctx.workspacePath });
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new SweetpadRpcError(ERROR_CODES.SIMULATOR_OP_FAILED, `Failed to boot ${sim.name}: ${message}`);
  }
  await ctx.destinationsManager.refreshSimulators();
  const refreshed = await findSimulator(ctx, sim.udid);
  return { booted: true, alreadyRunning: false, simulator: refreshed };
};

export const simulatorStop: HandlerFn<
  { id?: string },
  { stopped: true; alreadyStopped: boolean; simulator: SimulatorEntity }
> = async (params, ctx) => {
  const id = requireString(params?.id, "simulator.stop", "id");
  const sim = await findSimulator(ctx, id);
  if (sim.state !== "Booted") return { stopped: true, alreadyStopped: true, simulator: sim };
  try {
    await execa("xcrun", ["simctl", "shutdown", sim.udid], { cwd: ctx.workspacePath });
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new SweetpadRpcError(ERROR_CODES.SIMULATOR_OP_FAILED, `Failed to shutdown ${sim.name}: ${message}`);
  }
  await ctx.destinationsManager.refreshSimulators();
  const refreshed = await findSimulator(ctx, sim.udid);
  return { stopped: true, alreadyStopped: false, simulator: refreshed };
};

export const simulatorRefresh: HandlerFn<unknown, { simulators: SimulatorEntity[] }> = async (_params, ctx) => {
  await ctx.destinationsManager.refreshSimulators();
  return { simulators: await loadSimulators(ctx) };
};
