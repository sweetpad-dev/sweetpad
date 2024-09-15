import type { SimulatorType } from "../simulators/types";
import type { DestinationType } from "./types";

export type DestinationOs =
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

export const SUPPORTED_DESTINATION_PLATFORMS: DestinationPlatform[] = [
  "iphoneos",
  "iphonesimulator",
  "watchsimulator",
  // "watchos",
  "macosx",
];

export const DESTINATION_TYPE_PRIORITY: DestinationType[] = [
  "iOSSimulator",
  "iOSDevice",
  "watchOSSimulator",
  // "watchOSDevice",
  "macOS",
];
export const SIMULATOR_TYPE_PRIORITY: SimulatorType[] = [
  "iPhone",
  "iPad",
  "AppleWatch",
  "AppleTV",
  "AppleVision",
  "iPod",
];
