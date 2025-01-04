import os from "node:os";
import type { DestinationPlatform } from "./constants";
import type { Destination } from "./types";

export function getMacOSArchitecture(): "arm64" | "x86_64" | null {
  const architecture = os.arch();

  switch (architecture) {
    case "arm64":
      return "arm64"; // Apple Silicon (M1, M2, etc.)
    case "x64":
      return "x86_64"; // Intel-based Mac
    default:
      return null;
  }
}

export function splitSupportedDestinatinos(options: {
  destinations: Destination[];
  supportedPlatforms: DestinationPlatform[] | undefined;
}): {
  supported: Destination[];
  unsupported: Destination[];
} {
  const { destinations, supportedPlatforms } = options;

  const supportedDestinations: Destination[] = [];
  const unsupportedDestinations: Destination[] = [];

  // If supportedPlatforms is undefined, we support all platforms
  if (supportedPlatforms === undefined) {
    return {
      supported: destinations,
      unsupported: [],
    };
  }
  for (const destination of destinations) {
    if (supportedPlatforms.includes(destination.platform)) {
      supportedDestinations.push(destination);
    } else {
      unsupportedDestinations.push(destination);
    }
  }

  return {
    supported: supportedDestinations,
    unsupported: unsupportedDestinations,
  };
}
