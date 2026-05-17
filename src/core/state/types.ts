import type { DestinationType, SelectedDestination } from "../destination/types";

export type LastLaunchedAppDeviceContext = {
  type: "device";
  appPath: string;
  appName: string;
  executableName?: string;
  bundleIdentifier: string;
  destinationId: string;
  destinationType: DestinationType;
};

export type LastLaunchedAppSimulatorContext = {
  type: "simulator";
  appPath: string;
  bundleIdentifier: string;
  simulatorUdid: string;
};

export type LastLaunchedAppMacOSContext = {
  type: "macos";
  appPath: string;
  bundleIdentifier: string;
};

export type LastLaunchedAppContext =
  | LastLaunchedAppDeviceContext
  | LastLaunchedAppSimulatorContext
  | LastLaunchedAppMacOSContext;

export type WorkspaceTypes = {
  "build.xcodeWorkspacePath": string;
  "build.xcodeProjectPath": string;
  "build.xcodeScheme": string;
  "build.xcodeConfiguration": string;
  "build.xcodeDestination": SelectedDestination;
  "build.xcodeDestinationsUsageStatistics": Record<string, number>;
  "build.xcodeDestinationsRecent": SelectedDestination[];
  "build.xcodeSdk": string;
  "build.lastLaunchedApp": LastLaunchedAppContext;
  "build.xcodeBuildServerAutogenreateInfoShown": boolean;
  "build.lspDiagnosticsEnabled": boolean;
  "build.lspDiagnosticsPostReloadAction": "enabled" | "disabled";
  "testing.xcodeTarget": string;
  "testing.xcodeConfiguration": string;
  "testing.xcodeDestination": SelectedDestination;
  "testing.xcodeScheme": string;
};

export type WorkspaceStateKey = keyof WorkspaceTypes;

export interface WorkspaceState {
  get<K extends WorkspaceStateKey>(key: K): WorkspaceTypes[K] | undefined;
  update<K extends WorkspaceStateKey>(key: K, value: WorkspaceTypes[K] | undefined): void;
  reset(): void;
}
