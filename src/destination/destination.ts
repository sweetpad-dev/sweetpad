import { OS, ArchType, Platform } from "../common/destinationTypes";

export interface Destination {
  udid: string | undefined;
  state: "Booted" | "Shutdown";
  name: string;
  os: OS;
  version: string | undefined;
  isSimulator: boolean;
  isAvailableForBuild: boolean;
  isAvailableForRun: boolean;
  archTypes: ArchType[] | undefined;

  getPlatform(): Platform;
}

export class SelectableDestination implements Destination {
  udid: string | undefined;
  state: "Booted" | "Shutdown";
  name: string;
  os: OS;
  version: string | undefined;
  isSimulator: boolean;
  isAvailableForBuild: boolean;
  isAvailableForRun: boolean;
  archTypes: ArchType[] | undefined;

  constructor(options: {
    udid: string | undefined;
    state: "Booted" | "Shutdown";
    name: string;
    os: OS;
    version: string | undefined;
    isSimulator: boolean;
    isAvailableForBuild: boolean;
    isAvailableForRun: boolean;
    archTypes: ArchType[] | undefined;
  }) {
    this.udid = options.udid;
    this.state = options.state;
    this.name = options.name;
    this.os = options.os;
    this.version = options.version;
    this.isSimulator = options.isSimulator;
    this.isAvailableForBuild = options.isAvailableForBuild;
    this.isAvailableForRun = options.isAvailableForRun;
    this.archTypes = options.archTypes;
  }

  // func to getPlatform
  getPlatform(): Platform {
    switch (this.os) {
      case OS.iOS:
        return this.isSimulator ? Platform.iphonesimulator : Platform.iphoneos;
      case OS.watchOS:
        return this.isSimulator ? Platform.watchsimulator : Platform.watchos;
      case OS.macOS:
        return Platform.macosx;
    }
  }
}
