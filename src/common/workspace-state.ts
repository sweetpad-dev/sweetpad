import type * as vscode from "vscode";

import type { DestinationType, SelectedDestination } from "../destination/types";

export type LastLaunchedAppDeviceContext = {
  type: "device";
  appPath: string; // Example: "/Users/username/Library/Developer/Xcode/DerivedData/MyApp-..."
  appName: string; // Example: "MyApp.app"
  executableName?: string; // Example: "MyApp" — CFBundleExecutable, process name in os_log
  bundleIdentifier: string; // Example: "com.example.MyApp"
  destinationId: string; // Example: "00008030-001A0A3E0A68002E"
  destinationType: DestinationType; // Example: "iOS"
};

export type LastLaunchedAppSimulatorContext = {
  type: "simulator";
  appPath: string;
  bundleIdentifier: string; // Example: "com.example.MyApp"
  simulatorUdid: string; // Example: "00000000-0000-0000-0000-000000000000"
};

export type LastLaunchedAppMacOSContext = {
  type: "macos";
  appPath: string;
  bundleIdentifier: string; // Example: "com.example.MyApp"
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
  "build.xcodeDestinationsUsageStatistics": Record<string, number>; // destinationId -> usageCount
  "build.xcodeDestinationsRecent": SelectedDestination[];
  "build.xcodeSdk": string;
  "build.lastLaunchedApp": LastLaunchedAppContext;
  "build.xcodeBuildServerAutogenreateInfoShown": boolean;
  "testing.xcodeTarget": string;
  "testing.xcodeConfiguration": string;
  "testing.xcodeDestination": SelectedDestination;
  "testing.xcodeScheme": string;
};

export type WorkspaceStateKey = keyof WorkspaceTypes;

const PREFIX = "sweetpad.";

export class WorkspaceStateService {
  constructor(private readonly vscodeContext: vscode.ExtensionContext) {}

  get<K extends WorkspaceStateKey>(key: K): WorkspaceTypes[K] | undefined {
    return this.vscodeContext.workspaceState.get(`${PREFIX}${key}`);
  }

  update<K extends WorkspaceStateKey>(key: K, value: WorkspaceTypes[K] | undefined): void {
    this.vscodeContext.workspaceState.update(`${PREFIX}${key}`, value);
  }

  /**
   * Remove all sweetpad.* keys from workspace state.
   */
  reset(): void {
    for (const key of this.vscodeContext.workspaceState.keys()) {
      if (key?.startsWith(PREFIX)) {
        this.vscodeContext.workspaceState.update(key, undefined);
      }
    }
  }
}
