import type { IDestination } from "../destination/types";

export type SimulatorType = "iPhone" | "iPad" | "iPod" | "AppleTV" | "AppleWatch" | "AppleVision";

export type SimulatorOS = "xrOS" | "iOS" | "tvOS" | "watchOS";

export class iOSSimulatorDestination implements IDestination {
  type = "iOSSimulator" as const;
  typeLabel = "iOS Simulator";
  platform = "iphonesimulator" as const;

  // ex. 10D6D4A3-3A3D-4D3D-8D3D-3D3D3D3D3D3D
  public udid: string;
  public isAvailable: boolean;
  public state: "Booted" | "Shutdown";
  // ex. iPhone 12 Pro Max
  public name: string;
  // ex. iPhone | iPad
  public simulatorType: SimulatorType;
  // NOTE: iPadOS is just iOS
  public os: "iOS";
  // ex. 15.2
  public osVersion: string;
  // ex. com.apple.CoreSimulator.SimDeviceType.iPhone-12-Pro-Max
  public rawDeviceTypeIdentifier: string;
  // ex. com.apple.CoreSimulator.SimRuntime.iOS-15-2
  public rawRuntime: string;

  constructor(options: {
    udid: string;
    isAvailable: boolean;
    state: "Booted" | "Shutdown";
    name: string;
    simulatorType: SimulatorType;
    os: "iOS";
    osVersion: string;
    rawDeviceTypeIdentifier: string;
    rawRuntime: string;
  }) {
    this.udid = options.udid;
    this.isAvailable = options.isAvailable;
    this.state = options.state;
    this.name = options.name;
    this.simulatorType = options.simulatorType;
    this.os = options.os;
    this.osVersion = options.osVersion;
    this.rawDeviceTypeIdentifier = options.rawDeviceTypeIdentifier;
    this.rawRuntime = options.rawRuntime;
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
}

export class watchOSSimulatorDestination implements IDestination {
  type = "watchOSSimulator" as const;
  typeLabel = "watchOS";
  platform = "watchsimulator" as const;

  // ex. 10D6D4A3-3A3D-4D3D-8D3D-3D3D3D3D3D3D
  public udid: string;
  public isAvailable: boolean;
  public state: "Booted" | "Shutdown";
  // ex. Apple Watch Series 5 - 40mm
  public name: string;
  public os: "watchOS";
  // ex. 8.5
  public osVersion: string;
  // ex. com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-5-40mm
  public rawDeviceTypeIdentifier: string;
  // ex. com.apple.CoreSimulator.SimRuntime.watchOS-8-5
  public rawRuntime: string;

  constructor(options: {
    udid: string;
    isAvailable: boolean;
    state: "Booted" | "Shutdown";
    name: string;
    os: "watchOS";
    osVersion: string;
    rawDeviceTypeIdentifier: string;
    rawRuntime: string;
  }) {
    this.udid = options.udid;
    this.isAvailable = options.isAvailable;
    this.state = options.state;
    this.name = options.name;
    this.os = options.os;
    this.osVersion = options.osVersion;
    this.rawDeviceTypeIdentifier = options.rawDeviceTypeIdentifier;
    this.rawRuntime = options.rawRuntime;
  }

  get id(): string {
    return `watchossimulator-${this.udid}`;
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
      return "sweetpad-device-watch";
    }
    return "sweetpad-device-watch-pause";
  }
}

export class tvOSSimulatorDestination implements IDestination {
  type = "tvOSSimulator" as const;
  typeLabel = "tvOS";
  platform = "appletvsimulator" as const;

  // ex. 10D6D4A3-3A3D-4D3D-8D3D-3D3D3D3D3D3D
  public udid: string;
  public isAvailable: boolean;
  public state: "Booted" | "Shutdown";
  // ex. Apple TV 4K (2nd generation)
  public name: string;
  public os: "tvOS";
  // ex. 15.2
  public osVersion: string;
  // ex. com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-2nd-gen
  public rawDeviceTypeIdentifier: string;
  // ex. com.apple.CoreSimulator.SimRuntime.tvOS-15-2
  public rawRuntime: string;

  constructor(options: {
    udid: string;
    isAvailable: boolean;
    state: "Booted" | "Shutdown";
    name: string;
    os: "tvOS";
    osVersion: string;
    rawDeviceTypeIdentifier: string;
    rawRuntime: string;
  }) {
    this.udid = options.udid;
    this.isAvailable = options.isAvailable;
    this.state = options.state;
    this.name = options.name;
    this.os = options.os;
    this.osVersion = options.osVersion;
    this.rawDeviceTypeIdentifier = options.rawDeviceTypeIdentifier;
    this.rawRuntime = options.rawRuntime;
  }

  get id(): string {
    return `tvossimulator-${this.udid}`;
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
    return "sweetpad-device-tv-old";
  }
}

export class visionOSSimulatorDestination implements IDestination {
  type = "visionOSSimulator" as const;
  typeLabel = "Apple Vision";
  platform = "xrsimulator" as const;

  // ex. 10D6D4A3-3A3D-4D3D-8D3D-3D3D3D3D3D3D
  public udid: string;
  public isAvailable: boolean;
  public state: "Booted" | "Shutdown";
  // ex. Apple Vision
  public name: string;
  public os: "xrOS";
  // ex. 15.2
  public osVersion: string;
  // ex. com.apple.CoreSimulator.SimDeviceType.Apple-Vision
  public rawDeviceTypeIdentifier: string;
  // ex. com.apple.CoreSimulator.SimRuntime.iOS-15-2
  public rawRuntime: string;

  constructor(options: {
    udid: string;
    isAvailable: boolean;
    state: "Booted" | "Shutdown";
    name: string;
    os: "xrOS";
    osVersion: string;
    rawDeviceTypeIdentifier: string;
    rawRuntime: string;
  }) {
    this.udid = options.udid;
    this.isAvailable = options.isAvailable;
    this.state = options.state;
    this.name = options.name;
    this.os = options.os;
    this.osVersion = options.osVersion;
    this.rawDeviceTypeIdentifier = options.rawDeviceTypeIdentifier;
    this.rawRuntime = options.rawRuntime;
  }

  get id(): string {
    return `visionsimulator-${this.udid}`;
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
    return "sweetpad-cardboards";
  }
}

export type SimulatorDestination =
  | iOSSimulatorDestination
  | watchOSSimulatorDestination
  | visionOSSimulatorDestination
  | tvOSSimulatorDestination;
