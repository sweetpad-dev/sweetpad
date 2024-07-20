export enum DestinationOS {
  iOS = "iOS",
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
