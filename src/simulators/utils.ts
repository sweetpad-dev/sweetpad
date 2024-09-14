import type { ExtensionContext } from "../common/commands";
import { ExtensionError } from "../common/errors";
import type { iOSSimulatorDestination } from "./types";

export async function getSimulatorByUdid(
  context: ExtensionContext,
  options: {
    udid: string;
    refresh: boolean;
  },
): Promise<iOSSimulatorDestination> {
  const simulators = await context.destinationsManager.getiOSSimulators({
    refresh: options.refresh ?? false,
  });
  for (const simulator of simulators) {
    if (simulator.udid === options.udid) {
      return simulator;
    }
  }
  throw new ExtensionError("Simulator not found", { context: { udid: options.udid } });
}
