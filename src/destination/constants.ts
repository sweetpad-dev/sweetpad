import type { iOSSimulatorDeviceType } from "../common/cli/scripts";
import type { DeviceCtlDeviceType } from "../common/xcode/devicectl";
import type { DestinationType } from "./types";

export type DestinationOS =
  | "iOS" // also includes iPadOS and visionOS/xrOS
  | "watchOS"
  | "macOS";

export type DestinationPlatform =
  | "macosx" // macOS
  | "iphoneos" // iOS Device
  | "iphonesimulator" // iOS Simulator
  | "watchos" // watchOS Device
  | "watchsimulator" // watchOS Simulator
  | "xros" // visionOS/xrOS Device
  | "xrsimulator"; // visionOS/xrOS Simulator

export const SUPPORTED_DESTINATION_IOS_PLATFORMS: DestinationPlatform[] = [
  "iphoneos",
  "iphonesimulator",
];

export const SUPPORTED_DESTINATION_PLATFORMS: DestinationPlatform[] = [
  ...SUPPORTED_DESTINATION_IOS_PLATFORMS,
  "macosx",
];


export const DESTINATION_TYPE_PRIORITY: DestinationType[] = ["iOSSimulator", "iOSDevice", "macOS"];
export const DESTINATION_IOS_SIMULATOR_DEVICE_TYPE_PRIORITY: iOSSimulatorDeviceType[] = [
  "iPhone",
  "iPad",
  "AppleWatch",
  "AppleTV",
  "AppleVision",
];
export const DESTINATION_IOS_DEVICE_TYPE_PRIORITY: DeviceCtlDeviceType[] = ["iPhone", "iPad"];
