import { DestinationPlatform } from "./constants";

export function isSimulator(platform: DestinationPlatform): boolean {
  return platform === DestinationPlatform.iphonesimulator || platform === DestinationPlatform.watchsimulator;
}

export function getDestinationName(platform: DestinationPlatform): string {
  switch (platform) {
    case DestinationPlatform.macosx:
      return "macOS";
    case DestinationPlatform.iphoneos:
      return "iOS";
    case DestinationPlatform.iphonesimulator:
      return "iOS Simulator";
    case DestinationPlatform.watchos:
      return "watchOS";
    case DestinationPlatform.watchsimulator:
      return "watchOS Simulator";
  }
}
