

export enum OS {
    iOS = "iOS",
    watchOS = "watchOS",
    macOS = "macOS",
  }

  export enum Platform {
    macosx = "macosx",
    iphoneos = "iphoneos",
    iphonesimulator = "iphonesimulator",
    watchos = "watchos",
    watchsimulator = "watchsimulator",
  }

  export function getOS(platform: Platform): OS {
    switch (platform) {
      case Platform.macosx:
        return OS.macOS;
      case Platform.iphoneos:
      case Platform.iphonesimulator:
        return OS.iOS;
      case Platform.watchos:
      case Platform.watchsimulator:
        return OS.watchOS;
    }
  }
  
  export function getDestinationName(platform: Platform): string {
    switch (platform) {
      case Platform.macosx:
        return "macOS";
      case Platform.iphoneos:
        return "iOS";
      case Platform.iphonesimulator:
        return "iOS Simulator";
      case Platform.watchos:
        return "watchOS";
      case Platform.watchsimulator:
        return "watchOS Simulator";
    }
  }
  
  export enum ArchType {
    x86_64 = "x86_64",
    arm64 = "arm64",
  }
  