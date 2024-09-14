import type { iOSSimulatorDeviceType } from "../common/cli/scripts";
import type { DestinationOs } from "../destination/constants";
import type { IDestination } from "../destination/types";

export class iOSSimulatorDestination implements IDestination {
  type = "iOSSimulator" as const;
  typeLabel = "iOS Simulator";
  platform = "iphonesimulator" as const;

  public udid: string;
  public isAvailable: boolean;
  public state: "Booted" | "Shutdown";

  public deviceType: iOSSimulatorDeviceType | null;
  public name: string;
  public runtime: string;
  public osVersion: string;
  public osType: DestinationOs;

  constructor(options: {
    udid: string;
    isAvailable: boolean;
    state: "Booted" | "Shutdown";
    name: string;
    rawDeviceType: string;
    runtime: string;
  }) {
    this.udid = options.udid;
    this.isAvailable = options.isAvailable;
    this.state = options.state;
    this.name = options.name;
    this.deviceType = iOSSimulatorDestination.parseDeviceType(options.rawDeviceType);
    this.runtime = options.runtime;

    // iOS-14-5 => 14.5
    const rawiOSVersion = options.runtime.split(".").slice(-1)[0];
    this.osVersion = rawiOSVersion.replace(/^(\w+)-(\d+)-(\d+)$/, "$2.$3");

    // "com.apple.CoreSimulator.SimRuntime.iOS-16-4"
    // "com.apple.CoreSimulator.SimRuntime.WatchOS-8-0"
    // extract iOS, tvOS, watchOS
    const regex = /com\.apple\.CoreSimulator\.SimRuntime\.(iOS|tvOS|watchOS)-\d+-\d+/;
    const match = this.runtime.match(regex);
    this.osType = match ? (match[1] as DestinationOs) : "iOS";
  }

  get id(): string {
    return `iossimulator-${this.udid}`;
  }

  get isBooted(): boolean {
    return this.state === "Booted";
  }

  get label(): string {
    // iPhone 12 Pro Max (14.5)
    return `${this.name} (${this.osVersion})`;
  }

  get quickPickDetails(): string {
    return `Type: ${this.typeLabel}, Version: ${this.osVersion}, ID: ${this.udid.toLocaleLowerCase()}`;
  }

  get icon(): string {
    if (this.isBooted) {
      return "sweetpad-device-mobile";
    }
    return "sweetpad-device-mobile-pause";
  }

  static parseDeviceType(rawDeviceType: string): iOSSimulatorDeviceType | null {
    // examples:
    // - "com.apple.CoreSimulator.SimDeviceType.Apple-Vision-Pro"
    // - "com.apple.CoreSimulator.SimDeviceType.iPhone-8"
    // - "com.apple.CoreSimulator.SimDeviceType.iPhone-11-Pro"
    // - "com.apple.CoreSimulator.SimDeviceType.iPod-touch--7th-generation-"
    // - "com.apple.CoreSimulator.SimDeviceType.Apple-TV-1080p"
    // - "com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-3rd-generation-4K"
    // - "com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-5-40mm"
    // common prefix amoung all device types (hope so)
    const prefix = "com.apple.CoreSimulator.SimDeviceType.";
    if (!rawDeviceType?.startsWith(prefix)) {
      return null;
    }

    const deviceType = rawDeviceType.slice(prefix.length);
    if (!deviceType) {
      return null;
    }
    if (deviceType.startsWith("iPhone")) {
      return "iPhone";
    }
    if (deviceType.startsWith("iPad")) {
      return "iPad";
    }
    if (deviceType.startsWith("iPod")) {
      return "iPod";
    }
    if (deviceType.startsWith("Apple-TV")) {
      return "AppleTV";
    }
    if (deviceType.startsWith("Apple-Watch")) {
      return "AppleWatch";
    }
    if (deviceType.startsWith("Apple-Vision")) {
      return "AppleVision";
    }
    return null;
  }
}
