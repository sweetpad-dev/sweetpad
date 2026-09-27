import { beforeEach, describe, expect, it, vi } from "vitest";

import { iOSSimulatorDestination } from "../../simulators/types";
import type { RpcContext } from "./context";
import { destinationList } from "./destination";
import { simulatorList, simulatorStart } from "./simulator";
import { simulatorScreenshot } from "./simulator-app";

const execaCalls: string[][] = [];

vi.mock("execa", () => ({
  execa: (_file: string, args: string[]) => {
    execaCalls.push(args);
    return Promise.resolve({ stdout: "" });
  },
}));

function simulator(state: "Booted" | "Shutdown"): iOSSimulatorDestination {
  return new iOSSimulatorDestination({
    udid: "BB655012-31A9-4907-AC25-4D25C201988F",
    isAvailable: true,
    state: state,
    name: "iPhone 17",
    simulatorType: "iPhone",
    os: "iOS",
    osVersion: "27.0",
    rawDeviceTypeIdentifier: "com.apple.CoreSimulator.SimDeviceType.iPhone-17",
    rawRuntime: "com.apple.CoreSimulator.SimRuntime.iOS-27-0",
  });
}

/**
 * A destinations manager whose cache still says the simulator is shut down, while simctl
 * (what a refresh reads) says it has booted since: Simulator.app or another tool started it.
 */
function makeCtx(): RpcContext {
  let current = [simulator("Shutdown")];
  const destinationsManager = {
    getSimulators: vi.fn(async () => current),
    refreshSimulators: vi.fn(async () => {
      current = [simulator("Booted")];
      return current;
    }),
    getDestinations: vi.fn(async () => current),
    getSelectedXcodeDestinationForBuild: vi.fn(() => undefined),
  };
  return {
    workspacePath: "/tmp/ws",
    destinationsManager: destinationsManager as unknown as RpcContext["destinationsManager"],
  } as unknown as RpcContext;
}

describe("simulator RPCs read the state simctl reports now", () => {
  beforeEach(() => {
    execaCalls.length = 0;
  });

  it("lists a simulator booted since the last refresh as booted", async () => {
    const { simulators } = await simulatorList({ state: "Booted" }, makeCtx());

    expect(simulators.map((s) => s.state)).toEqual(["Booted"]);
  });

  it("reports it already running instead of booting it again", async () => {
    const result = await simulatorStart({ id: "BB655012-31A9-4907-AC25-4D25C201988F" }, makeCtx());

    expect(result.alreadyRunning).toBe(true);
    expect(execaCalls).toEqual([]);
  });

  it("acts on it without refusing it as not booted", async () => {
    await simulatorScreenshot({ udid: "BB655012-31A9-4907-AC25-4D25C201988F", path: "/tmp/shot.png" }, makeCtx());

    expect(execaCalls[0]?.slice(0, 3)).toEqual(["simctl", "io", "BB655012-31A9-4907-AC25-4D25C201988F"]);
  });

  it("filters destinations on the current state", async () => {
    const { destinations } = await destinationList({ booted: true }, makeCtx());

    expect(destinations.map((d) => d.simulatorState)).toEqual(["Booted"]);
  });
});
