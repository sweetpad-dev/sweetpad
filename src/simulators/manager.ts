import events from "node:events";
import { getSimulators } from "../common/cli/scripts";
import { iOSSimulatorDestination } from "./types";

type IEventMap = {
  updated: [];
};

/**
 * Simulator manager that gets the list of iOS simuulators, including iOS, watchOS, and tvOS.
 */
export class SimulatorsManager {
  private cache: iOSSimulatorDestination[] | undefined = undefined;

  private emitter = new events.EventEmitter<IEventMap>();

  on(event: "updated", listener: () => void): void {
    this.emitter.on(event, listener);
  }

  private async fetchSimulators(): Promise<iOSSimulatorDestination[]> {
    const output = await getSimulators();
    const simulators = Object.entries(output.devices)
      .flatMap(([key, value]) =>
        value.map((simulator) => {
          return new iOSSimulatorDestination({
            udid: simulator.udid,
            isAvailable: simulator.isAvailable,
            state: simulator.state as "Booted",
            name: simulator.name,
            rawDeviceType: simulator.deviceTypeIdentifier,
            runtime: key,
          });
        }),
      )
      .filter((simulator) => simulator.isAvailable)
      // temporary filter to only show iOS simulators
      .filter((simulator) => simulator.osType === "iOS");
    return simulators;
  }

  async refresh(): Promise<iOSSimulatorDestination[]> {
    this.cache = await this.fetchSimulators();
    this.emitter.emit("updated");
    return this.cache;
  }

  async getSimulators(options?: { refresh?: boolean }): Promise<iOSSimulatorDestination[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh();
    }
    return this.cache;
  }
}
