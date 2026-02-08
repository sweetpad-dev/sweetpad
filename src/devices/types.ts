import type { DeviceCtlDevice } from "../common/xcode/devicectl";
import type { IDestination } from "../destination/types";
import { supportsDevicectl } from "./utils";

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
    const osVersion = this.osVersion;
    if (osVersion === "Unknown") {
      return this.name;
    }
    return `${this.name} (${osVersion})`;
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

  /**
   * The legacy UDID format used by xcodebuild destination
   * For older devices, this comes from xcdevice; for newer devices, from devicectl
   */
  get udid() {
    return this.device.hardwareProperties.udid ?? this.device.identifier ?? "unknown";
  }

  /**
   * The devicectl identifier used for devicectl commands
   * This is always the identifier from devicectl list
   */
  get devicectlId() {
    return this.device.identifier;
  }

  get name() {
    return (
      this.device.deviceProperties.name ??
      this.device.hardwareProperties.marketingName ??
      this.device.hardwareProperties.productType ??
      "Unknown Device"
    );
  }

  get osVersion() {
    return this.device.deviceProperties.osVersionNumber ?? "Unknown";
  }

  get state(): "connected" | "disconnected" | "unavailable" {
    return this.device.connectionProperties.tunnelState;
  }

  get deviceType() {
    return this.device.hardwareProperties.deviceType;
  }

  /**
   * Check if the device supports devicectl (iOS 17+)
   * Older devices (iOS < 17) need to use ios-deploy instead
   */
  get supportsDevicectl(): boolean {
    return supportsDevicectl(this.device.deviceProperties.osVersionNumber, 17);
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

  /**
   * The legacy UDID format used by xcodebuild destination
   * For older devices, this comes from xcdevice; for newer devices, from devicectl
   */
  get udid() {
    return this.device.hardwareProperties.udid ?? this.device.identifier ?? "unknown";
  }

  /**
   * The devicectl identifier used for devicectl commands
   * This is always the identifier from devicectl list
   */
  get devicectlId() {
    return this.device.identifier;
  }

  get name() {
    return (
      this.device.deviceProperties.name ??
      this.device.hardwareProperties.marketingName ??
      this.device.hardwareProperties.productType ??
      "Unknown Device"
    );
  }

  get label(): string {
    const osVersion = this.osVersion;
    if (osVersion === "Unknown") {
      return this.name;
    }
    return `${this.name} (${osVersion})`;
  }

  get osVersion() {
    return this.device.deviceProperties.osVersionNumber ?? "Unknown";
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

  /**
   * Check if the device supports devicectl (watchOS 10+)
   * Older devices (watchOS < 10) need to use alternative methods
   */
  get supportsDevicectl(): boolean {
    return supportsDevicectl(this.device.deviceProperties.osVersionNumber, 10);
  }
}

export class tvOSDeviceDestination implements IDestination {
  type = "tvOSDevice" as const;
  typeLabel = "tvOS Device";
  platform = "appletvos" as const;

  constructor(public device: DeviceCtlDevice) {
    this.device = device;
  }

  get id(): string {
    return `tvosdevice-${this.udid}`;
  }

  get icon(): string {
    return "sweetpad-device-tv-old";
  }

  /**
   * The legacy UDID format used by xcodebuild destination
   * For older devices, this comes from xcdevice; for newer devices, from devicectl
   */
  get udid() {
    return this.device.hardwareProperties.udid ?? this.device.identifier ?? "unknown";
  }

  /**
   * The devicectl identifier used for devicectl commands
   * This is always the identifier from devicectl list
   */
  get devicectlId() {
    return this.device.identifier;
  }

  get name() {
    return (
      this.device.deviceProperties.name ??
      this.device.hardwareProperties.marketingName ??
      this.device.hardwareProperties.productType ??
      "Unknown Device"
    );
  }

  get label(): string {
    const osVersion = this.osVersion;
    if (osVersion === "Unknown") {
      return this.name;
    }
    return `${this.name} (${osVersion})`;
  }

  get osVersion() {
    return this.device.deviceProperties.osVersionNumber ?? "Unknown";
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

  /**
   * Check if the device supports devicectl (tvOS 17+)
   * Older devices (tvOS < 17) need to use alternative methods
   */
  get supportsDevicectl(): boolean {
    return supportsDevicectl(this.device.deviceProperties.osVersionNumber, 17);
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

  /**
   * The legacy UDID format used by xcodebuild destination
   * For older devices, this comes from xcdevice; for newer devices, from devicectl
   */
  get udid() {
    return this.device.hardwareProperties.udid ?? this.device.identifier ?? "unknown";
  }

  /**
   * The devicectl identifier used for devicectl commands
   * This is always the identifier from devicectl list
   */
  get devicectlId() {
    return this.device.identifier;
  }

  get name() {
    return (
      this.device.deviceProperties.name ??
      this.device.hardwareProperties.marketingName ??
      this.device.hardwareProperties.productType ??
      "Unknown Device"
    );
  }

  get label(): string {
    const osVersion = this.osVersion;
    if (osVersion === "Unknown") {
      return this.name;
    }
    return `${this.name} (${osVersion})`;
  }

  get osVersion() {
    return this.device.deviceProperties.osVersionNumber ?? "Unknown";
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

  /**
   * Check if the device supports devicectl (visionOS 1+)
   * All visionOS devices support devicectl as it's a newer platform
   */
  get supportsDevicectl(): boolean {
    return supportsDevicectl(this.device.deviceProperties.osVersionNumber, 1);
  }
}

export type DeviceDestination =
  | iOSDeviceDestination
  | watchOSDeviceDestination
  | tvOSDeviceDestination
  | visionOSDeviceDestination;
