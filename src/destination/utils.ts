import os from "node:os";

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
