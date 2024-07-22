import { iOSSimulator, getSimulators } from "../common/cli/scripts";
import events from "events";

type IEventMap = {
  updated: [];
};

/**
 * Simulator manager that gets the list of iOS simuulators, including iOS, watchOS, and tvOS.
 */
export class SimulatorsManager {
  private cache: iOSSimulator[] | undefined = undefined;

  private emitter = new events.EventEmitter<IEventMap>();

  on(event: "updated", listener: () => void): void {
    this.emitter.on(event, listener);
  }

  async refresh(): Promise<iOSSimulator[]> {
    this.cache = await getSimulators();
    this.emitter.emit("updated");
    return this.cache;
  }

  async getSimulators(options?: { refresh?: boolean }): Promise<iOSSimulator[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh();
    }
    return this.cache;
  }
}
