import events from "node:events";
import { type SimulatorOutput, getSimulators } from "../common/cli/scripts";
import { commonLogger } from "../common/logger";
import { assertUnreachable } from "../common/types";
import { type SimulatorDestination, iOSSimulatorDestination, watchOSSimulatorDestination } from "./types";
import { parseDeviceTypeIdentifier, parseSimulatorRuntime } from "./utils";

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
   * Convert the raw data from the system to a simulator destinations: iOSDestination, watchOSDestination, etc.
   */
  private prepareSimulator(rawRuntime: string, simulator: SimulatorOutput): SimulatorDestination | null {
    const simulatorType = parseDeviceTypeIdentifier(simulator.deviceTypeIdentifier);
    if (!simulatorType) {
      commonLogger.log("Can not parse device type", {
        runtime: rawRuntime,
        simulator: simulator,
      });
      return null;
    }

    const runtime = parseSimulatorRuntime(rawRuntime);
    if (!runtime) {
      commonLogger.log("Can not parse runtime", {
        runtime: rawRuntime,
        simulator: simulator,
      });
      return null;
    }

    if (runtime.os === "iOS") {
      // NOTE: iPadOS is just a variation of iOS, so we can use the same class.
      return new iOSSimulatorDestination({
        udid: simulator.udid,
        isAvailable: simulator.isAvailable,
        state: simulator.state as "Booted",
        name: simulator.name,
        simulatorType: simulatorType,
        os: runtime.os,
        osVersion: runtime.version,
        rawDeviceTypeIdentifier: simulator.deviceTypeIdentifier,
        rawRuntime: rawRuntime,
      });
    }
    if (runtime.os === "watchOS") {
      return new watchOSSimulatorDestination({
        udid: simulator.udid,
        isAvailable: simulator.isAvailable,
        state: simulator.state as "Booted",
        name: simulator.name,
        os: runtime.os,
        osVersion: runtime.version,
        rawDeviceTypeIdentifier: simulator.deviceTypeIdentifier,
        rawRuntime: rawRuntime,
      });
    }
    if (runtime.os === "tvOS") {
      // todo: add tvOS simulator support
      return null;
    }
    if (runtime.os === "xrOS") {
      // todo: add xrOS simulator support
      return null;
    }
    assertUnreachable(runtime.os);
  }

  /**
   * Fetch the list of simulators from the system. It returns iOS, watchOS, and other types of simulators.
   */
  private async fetchSimulators(): Promise<SimulatorDestination[]> {
    const output = await getSimulators();
    const simulators = Object.entries(output.devices)
      .flatMap(([key, simualtors]) => simualtors.map((simulator) => this.prepareSimulator(key, simulator)))
      .filter((simulator) => simulator !== null)
      .filter((simulator) => simulator.isAvailable);

    return simulators;
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
}
