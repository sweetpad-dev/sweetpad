import { iOSSimulatorDeviceType } from "../common/cli/scripts";
import { DeviceCtlDeviceType } from "../common/xcode/devicectl";
import { DestinationType } from "./types";


export type DestinationOS =
  "iOS" // also includes iPadOS and visionOS/xrOS 
  | "watchOS"
  | "macOS";


export type DestinationPlatform = "macosx" // macOS
  | "iphoneos" // iOS Device
  | "iphonesimulator" // iOS Simulator
  | "watchos" // watchOS Device
  | "watchsimulator" // watchOS Simulator
  | "xros" // visionOS/xrOS Device
  | "xrsimulator"; // visionOS/xrOS Simulator

export const SUPPORTED_DESTINATION_PLATFORMS = ["iphoneos", "iphonesimulator", "macosx"] satisfies DestinationPlatform[];

export const DESTINATION_TYPE_PRIORITY: DestinationType[] = ["iOSSimulator", "iOSDevice", "macOS"];
export const DESTINATION_IOS_SIMULATOR_DEVICE_TYPE_PRIORITY: iOSSimulatorDeviceType[] = [
  "iPhone",
  "iPad",
  "AppleWatch",
  "AppleTV",
  "AppleVision",
];
export const DESTINATION_IOS_DEVICE_TYPE_PRIORITY: DeviceCtlDeviceType[] = ["iPhone", "iPad"];
