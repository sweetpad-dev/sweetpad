import events from "node:events";

import * as sweetpadLib from "@sweetpad/native";

import { getSimulatorsJson } from "../common/cli/scripts";
import { commonLogger } from "../common/logger";
import {
  type SimulatorDestination,
  iOSSimulatorDestination,
  tvOSSimulatorDestination,
  visionOSSimulatorDestination,
  watchOSSimulatorDestination,
} from "./types";
import { parseDeviceTypeIdentifier } from "./utils";

type IEventMap = {
  updated: [];
};

/**
 * Simulator manager that gets the list of iOS simuulators, including iOS, watchOS, and tvOS.
 */
export class SimulatorsManager {
  private cache: SimulatorDestination[] | undefined = undefined;

  private emitter = new events.EventEmitter<IEventMap>();

  on(event: "updated", listener: () => void): void {
    this.emitter.on(event, listener);
  }

  /**
   * Convert a simulator the addon read from simctl to a destination: iOSDestination,
   * watchOSDestination, etc. A runtime OS this code has no destination class for is
   * logged and left out.
   */
  private prepareSimulator(simulator: sweetpadLib.SimctlSimulator): SimulatorDestination | null {
    const simulatorType = parseDeviceTypeIdentifier(simulator.deviceTypeIdentifier);
    if (!simulatorType) {
      commonLogger.log("Can not parse device type", {
        runtime: simulator.runtime,
        simulator: simulator,
      });
      return null;
    }

    const common = {
      udid: simulator.udid,
      isAvailable: true,
      state: simulator.state as "Booted",
      name: simulator.name,
      osVersion: simulator.osVersion,
      rawDeviceTypeIdentifier: simulator.deviceTypeIdentifier,
      rawRuntime: simulator.runtime,
    };
    switch (simulator.os) {
      case "iOS":
        // NOTE: iPadOS is just a variation of iOS, so we can use the same class.
        return new iOSSimulatorDestination({ ...common, simulatorType: simulatorType, os: "iOS" });
      case "watchOS":
        return new watchOSSimulatorDestination({ ...common, os: "watchOS" });
      case "tvOS":
        return new tvOSSimulatorDestination({ ...common, os: "tvOS" });
      case "xrOS":
        return new visionOSSimulatorDestination({ ...common, os: "xrOS" });
      default:
        commonLogger.log("Can not parse runtime", {
          runtime: simulator.runtime,
          simulator: simulator,
        });
        return null;
    }
  }

  /**
   * Fetch the list of simulators from the system. It returns iOS, watchOS, and other types of simulators.
   * The addon drops the unavailable ones.
   */
  private async fetchSimulators(): Promise<SimulatorDestination[]> {
    const simulators = sweetpadLib.parseSimulators(await getSimulatorsJson());
    return simulators.map((simulator) => this.prepareSimulator(simulator)).filter((simulator) => simulator !== null);
  }

  async refresh(): Promise<SimulatorDestination[]> {
    this.cache = await this.fetchSimulators();
    this.emitter.emit("updated");
    return this.cache;
  }

  async getSimulators(options?: { refresh?: boolean }): Promise<SimulatorDestination[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh();
    }
    return this.cache;
  }

  /**
   * What is already known, without going to simctl. For callers that have to answer
   * synchronously and can live with an empty list until the first fetch lands.
   */
  getCachedSimulators(): SimulatorDestination[] {
    return this.cache ?? [];
  }
}
