import { DevicesManager } from "../devices/manager";
import { SimulatorsManager } from "../simulators/manager";
import { Destination, SelectableDestination } from "./destination";
import { OS, ArchType } from "../common/destinationTypes";

export class DesintationManager {
    simulatorsManager: SimulatorsManager;
    devicesManager: DevicesManager;

    anyMac: Destination = new SelectableDestination({
        udid: undefined,
        state: "Booted",
        name: "Any Mac",
        os: OS.macOS,
        version: "Any",
        archTypes: [ArchType.x86_64, ArchType.arm64],
        isSimulator: false,
        isAvailableForBuild: true,
        isAvailableForRun: false,
    });

    myMac: Destination = new SelectableDestination({
        udid: undefined,
        state: "Booted",
        name: "My Mac",
        os: OS.macOS,
        version: "Any",
        archTypes: [ArchType.x86_64, ArchType.arm64],
        isSimulator: false,
        isAvailableForBuild: true,
        isAvailableForRun: true,
    });


    anyiOSSimulator: Destination = new SelectableDestination({
        udid: undefined,
        state: "Booted",
        name: "Any iOS Simulator",
        os: OS.iOS,
        version: "Any",
        archTypes: [ArchType.x86_64, ArchType.arm64],
        isSimulator: true,
        isAvailableForBuild: true,
        isAvailableForRun: false,
    });

    anyiOSDevice: Destination = new SelectableDestination({
        udid: undefined,
        state: "Booted",
        name: "Any iOS Device",
        os: OS.iOS,
        version: "Any",
        archTypes: [ArchType.arm64],
        isSimulator: false,
        isAvailableForBuild: true,
        isAvailableForRun: false,
    });

    anyWatchOSSimulator: Destination = new SelectableDestination({
        udid: undefined,
        state: "Booted",
        name: "Any WatchOS Simulator",
        os: OS.watchOS,
        version: "Any",
        archTypes: [ArchType.x86_64],
        isSimulator: true,
        isAvailableForBuild: true,
        isAvailableForRun: false,
    });

    anyWatchOSDevice: Destination = new SelectableDestination({
        udid: undefined,
        state: "Booted",
        name: "Any WatchOS Device",
        os: OS.watchOS,
        version: "Any",
        archTypes: [ArchType.arm64],
        isSimulator: false,
        isAvailableForBuild: true,
        isAvailableForRun: false,
    });

    constructor(options: {
        simulatorsManager: SimulatorsManager;
        devicesManager: DevicesManager;
    }) {
        this.simulatorsManager = options.simulatorsManager;
        this.devicesManager = options.devicesManager;
    }

    async getAvailableDestinations(
        osList: OS[]
    ): Promise<SelectableDestination[]> {
        const destinations: SelectableDestination[] = [];

        if (osList.includes(OS.iOS)) {
            destinations.push(this.anyiOSSimulator);
            destinations.push(this.anyiOSDevice);
        } else if (osList.includes(OS.watchOS)) {
            destinations.push(this.anyWatchOSSimulator);
            destinations.push(this.anyWatchOSDevice);
        } else if (osList.includes(OS.macOS)) {
            destinations.push(this.myMac);
            destinations.push(this.anyMac);
        }

        if (osList.includes(OS.iOS) || osList.includes(OS.watchOS)) {
            const simulators = await this.simulatorsManager.getSimulators( { refresh: true , filterOSTypes: osList });
            simulators.map((simulator) => {
                destinations.push(new SelectableDestination({
                    udid: simulator.udid,
                    state: simulator.state,
                    name: simulator.name,
                    os: simulator.runtimeType,
                    version: simulator.iosVersion,
                    archTypes: [ArchType.x86_64, ArchType.arm64],
                    isSimulator: true,
                    isAvailableForBuild: true,
                    isAvailableForRun: true,
                }));
            });
        }

        if (osList.includes(OS.iOS)) {
            const devices = await this.devicesManager.getDevices();
            devices.map((device) => {
                    destinations.push(new SelectableDestination({
                        udid: device.udid,
                        state: "Booted",
                        name: device.label,
                        os: OS.iOS,
                        version: undefined,
                        archTypes: [ArchType.arm64],
                        isSimulator: false,
                        isAvailableForBuild: true,
                        isAvailableForRun: true,
                    }));
            });
        }

        return destinations;
    }
}