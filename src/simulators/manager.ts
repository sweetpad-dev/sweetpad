import { IosSimulator, getSimulators } from "../common/cli/scripts";
import events from "events";

type SimulatorManagerEventTypes = {
  refresh: [];
};

export class SimulatorsManager {
  private cache: IosSimulator[] | undefined = undefined;

  private emitter = new events.EventEmitter<SimulatorManagerEventTypes>();

  on(event: "refresh", listener: () => void): void {
    this.emitter.on(event, listener);
  }

  async refresh(): Promise<IosSimulator[]> {
    this.cache = await getSimulators();
    this.emitter.emit("refresh");
    return this.cache;
  }

  async getSimulators(options?: { refresh?: boolean }): Promise<IosSimulator[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh();
    }
    return this.cache;
  }
}
