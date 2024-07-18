import { IosSimulator, getSimulators } from "../common/cli/scripts";
import events from "events";
import { OS } from "../common/destinationTypes";

type SimulatorManagerEventTypes = {
  refresh: [];
};

export class SimulatorsManager {
  private cache: IosSimulator[] | undefined = undefined;
  private cacheOSTypes: OS[] = [];

  private emitter = new events.EventEmitter<SimulatorManagerEventTypes>();

  on(event: "refresh", listener: () => void): void {
    this.emitter.on(event, listener);
  }

  async refresh(filterOSTypes: OS[]): Promise<IosSimulator[]> {
    this.cache = await getSimulators(filterOSTypes);
    this.cacheOSTypes = filterOSTypes;
    this.emitter.emit("refresh");
    return this.cache;
  }

  async getSimulators(options?: { refresh?: boolean; filterOSTypes: OS[] }): Promise<IosSimulator[]> {
    if (this.cache === undefined || options?.refresh || this.cacheOSTypes !== options?.filterOSTypes) {
      return await this.refresh(options?.filterOSTypes ?? [OS.iOS]);
    }
    return this.cache;
  }
}
