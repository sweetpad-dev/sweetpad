import { iOSSimulatorDeviceType } from "../common/cli/scripts";
import { DeviceCtlDeviceType } from "../common/xcode/devicectl";
import { DestinationType } from "./types";

export enum DestinationOS {
  iOS = "iOS", // also includes iPadOS and visionOS/xrOS
  watchOS = "watchOS",
  macOS = "macOS",
}

export enum DestinationPlatform {
  macosx = "macosx", // macOS
  iphoneos = "iphoneos", // iOS Device
  iphonesimulator = "iphonesimulator", // iOS Simulator
  watchos = "watchos", // watchOS Device
  watchsimulator = "watchsimulator",
}

export const SUPPORTED_DESTINATION_PLATFORMS = [DestinationPlatform.iphoneos, DestinationPlatform.iphonesimulator];

export const DESTINATION_TYPE_PRIORITY: DestinationType[] = ["iOSSimulator", "iOSDevice"];
export const DESTINATION_IOS_SIMULATOR_DEVICE_TYPE_PRIORITY: iOSSimulatorDeviceType[] = [
  "iPhone",
  "iPad",
  "AppleWatch",
  "AppleTV",
  "AppleVision",
];
export const DESTINATION_IOS_DEVICE_TYPE_PRIORITY: DeviceCtlDeviceType[] = ["iPhone", "iPad"];
