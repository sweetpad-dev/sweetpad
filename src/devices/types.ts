import type { DeviceCtlDevice } from "../common/xcode/devicectl";
import type { IDestination } from "../destination/types";

export class iOSDeviceDestination implements IDestination {
  type = "iOSDevice" as const;
  typeLabel = "iOS Device";
  platform = "iphoneos" as const;

  constructor(public device: DeviceCtlDevice) {
    this.device = device;
  }

  get id(): string {
    return `iosdevice-${this.udid}`;
  }

  get label(): string {
    // iPhone 12 Pro Max (14.5)
    return `${this.name} (${this.osVersion})`;
  }

  get quickPickDetails(): string {
    return `Type: ${this.typeLabel}, Version: ${this.osVersion}, ID: ${this.udid.toLocaleLowerCase()}`;
  }

  get isConnected(): boolean {
    return this.state === "connected";
  }

  get icon(): string {
    if (this.deviceType === "iPad") {
      if (this.isConnected) {
        return "sweetpad-device-ipad";
      }
      return "sweetpad-device-ipad-x";
    }
    if (this.deviceType === "iPhone") {
      if (this.isConnected) {
        return "sweetpad-device-mobile";
      }
      return "sweetpad-device-mobile-x";
    }
    return "sweetpad-device-mobile";
  }

  get udid() {
    return this.device.hardwareProperties.udid;
  }

  get name() {
    return this.device.deviceProperties.name;
  }

  get osVersion() {
    return this.device.deviceProperties.osVersionNumber;
  }

  get state(): "connected" | "disconnected" | "unavailable" {
    return this.device.connectionProperties.tunnelState;
  }

  get deviceType() {
    return this.device.hardwareProperties.deviceType;
  }
}

export class watchOSDeviceDestination implements IDestination {
  type = "watchOSDevice" as const;
  typeLabel = "watchOS Device";
  platform = "watchos" as const;

  constructor(public device: DeviceCtlDevice) {
    this.device = device;
  }

  get id(): string {
    return `watchosdevice-${this.udid}`;
  }

  get icon(): string {
    if (this.isConnected) {
      return "sweetpad-device-watch";
    }
    return "sweetpad-device-watch-pause";
  }

  get udid() {
    return this.device.hardwareProperties.udid;
  }

  get name() {
    return this.device.deviceProperties.name;
  }

  get label(): string {
    return `${this.name} (${this.osVersion})`;
  }

  get osVersion() {
    return this.device.deviceProperties.osVersionNumber;
  }

  get quickPickDetails(): string {
    return `Type: ${this.typeLabel}, Version: ${this.osVersion}, ID: ${this.udid.toLocaleLowerCase()}`;
  }

  get state(): "connected" | "disconnected" | "unavailable" {
    return this.device.connectionProperties.tunnelState;
  }

  get isConnected(): boolean {
    return this.state === "connected";
  }
}

export class visionOSDeviceDestination implements IDestination {
  type = "visionOSDevice" as const;
  typeLabel = "visionOS Device";
  platform = "xros" as const;

  constructor(public device: DeviceCtlDevice) {
    this.device = device;
  }

  get id(): string {
    return `visionosdevice-${this.udid}`;
  }

  get icon(): string {
    return "sweetpad-cardboards";
  }

  get udid() {
    return this.device.hardwareProperties.udid;
  }

  get name() {
    return this.device.deviceProperties.name;
  }

  get label(): string {
    return `${this.name} (${this.osVersion})`;
  }

  get osVersion() {
    return this.device.deviceProperties.osVersionNumber;
  }

  get quickPickDetails(): string {
    return `Type: ${this.typeLabel}, Version: ${this.osVersion}, ID: ${this.udid.toLocaleLowerCase()}`;
  }

  get state(): "connected" | "disconnected" | "unavailable" {
    return this.device.connectionProperties.tunnelState;
  }

  get isConnected(): boolean {
    return this.state === "connected";
  }
}

export type DeviceDestination = iOSDeviceDestination | watchOSDeviceDestination | visionOSDeviceDestination;
