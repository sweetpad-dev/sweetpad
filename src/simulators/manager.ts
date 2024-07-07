import { IosSimulator, SimDeviceOSType, getSimulators } from "../common/cli/scripts";
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

  async refresh(filterOSTypes: [SimDeviceOSType] = [SimDeviceOSType.iOS]): Promise<IosSimulator[]> {
    this.cache = await getSimulators(filterOSTypes);
    this.emitter.emit("refresh");
    return this.cache;
  }

  async getSimulators(options?: { refresh?: boolean , filterOSTypes: [SimDeviceOSType] }): Promise<IosSimulator[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh(options?.filterOSTypes ?? [SimDeviceOSType.iOS]);
    }
    return this.cache;
  }
}
