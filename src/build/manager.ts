import events from "node:events";
import * as path from "node:path";
import * as vscode from "vscode";
import {
  type XcodeScheme,
  generateBuildServerConfig,
  getIsXcodeBuildServerInstalled,
  getSchemes,
} from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { isFileExists } from "../common/files";
import { askXcodeWorkspacePath, getCurrentXcodeWorkspacePath, getWorkspacePath, restartSwiftLSP } from "./utils";
import { commonLogger } from "../common/logger";
import { BazelTreeItem } from "./tree";

type IEventMap = {
  updated: [];
  defaultSchemeForBuildUpdated: [scheme: string | undefined];
  defaultSchemeForTestingUpdated: [scheme: string | undefined];
  currentWorkspacePathUpdated: [workspacePath: string | undefined];
  selectedBazelTargetUpdated: [target: SelectedBazelTargetData | undefined];
};

// Serializable data for selected Bazel target (no circular references)
export interface SelectedBazelTargetData {
  targetName: string;
  targetType: "library" | "test" | "binary";
  buildLabel: string;
  testLabel?: string;
  packageName: string;
  packagePath: string;
  workspacePath: string;
}
type IEventKey = keyof IEventMap;

export class BuildManager {
  private cache: XcodeScheme[] | undefined = undefined;
  private emitter = new events.EventEmitter<IEventMap>();
  public _context: ExtensionContext | undefined = undefined;

  constructor() {
    this.on("defaultSchemeForBuildUpdated", (scheme: string | undefined) => {
      void this.generateXcodeBuildServerSettingsOnSchemeChange({
        scheme: scheme,
      });
    });
  }

  on<K extends IEventKey>(event: K, listener: (...args: IEventMap[K]) => void): void {
    this.emitter.on(event, listener as any); // todo: fix this any
  }

  set context(context: ExtensionContext) {
    this._context = context;
  }

  get context(): ExtensionContext {
    if (!this._context) {
      throw new Error("Context is not set");
    }
    return this._context;
  }

  async getSchemas(options?: { refresh?: boolean }): Promise<XcodeScheme[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh();
    }
    return this.cache;
  }

  async refresh(): Promise<XcodeScheme[]> {
    // Always get the latest workspace path from context
    const xcworkspace = getCurrentXcodeWorkspacePath(this.context);

    try {
      const scheme = await getSchemes({
        xcworkspace: xcworkspace,
      });

      this.cache = scheme;
      this.emitter.emit("updated");
      return this.cache;
    } catch (error) {
      // If there's an error getting schemes, return empty array
      commonLogger.error("Failed to get schemes", { error });
      this.cache = [];
      return [];
    }
  }

  getDefaultSchemeForBuild(): string | undefined {
    return this.context.getWorkspaceState("build.xcodeScheme");
  }

  getDefaultSchemeForTesting(): string | undefined {
    return this.context.getWorkspaceState("testing.xcodeScheme");
  }

  setDefaultSchemeForBuild(scheme: string | undefined): void {
    this.context.updateWorkspaceState("build.xcodeScheme", scheme);
    this.emitter.emit("defaultSchemeForBuildUpdated", scheme);
  }

  setCurrentWorkspacePath(workspacePath: string | undefined, skipRefresh: boolean = false): void {
    // Only update if the path has actually changed
    const currentPath = this.context.getWorkspaceState("build.xcodeWorkspacePath");
    if (currentPath === workspacePath) {
      return;
    }
    
    // Since workspace is changing, clear the scheme cache to prevent mixing schemes
    this.clearSchemesCache();
    
    this.context.updateWorkspaceState("build.xcodeWorkspacePath", workspacePath);
    this.emitter.emit("currentWorkspacePathUpdated", workspacePath);
    
    // Allow skipping the automatic refresh when needed
    if (!skipRefresh) {
      this.refresh();
    }
  }

  setDefaultSchemeForTesting(scheme: string | undefined): void {
    this.context.updateWorkspaceState("testing.xcodeScheme", scheme);
    this.emitter.emit("defaultSchemeForTestingUpdated", scheme);
  }

  getDefaultConfigurationForBuild(): string | undefined {
    return this.context.getWorkspaceState("build.xcodeConfiguration");
  }

  getDefaultConfigurationForTesting(): string | undefined {
    return this.context.getWorkspaceState("testing.xcodeConfiguration");
  }

  setDefaultConfigurationForBuild(configuration: string | undefined): void {
    this.context.updateWorkspaceState("build.xcodeConfiguration", configuration);
  }

  setDefaultConfigurationForTesting(configuration: string | undefined): void {
    this.context.updateWorkspaceState("testing.xcodeConfiguration", configuration);
  }

  clearSchemesCache(): void {
    this.cache = undefined;
  }

  /**
   * Every time the scheme changes, we need to rebuild the buildServer.json file
   * for providing the correct build settings to the LSP server.
   */
  async generateXcodeBuildServerSettingsOnSchemeChange(options: {
    scheme: string | undefined;
  }): Promise<void> {
    if (!options.scheme) {
      return;
    }

    const isEnabled = getWorkspaceConfig("xcodebuildserver.autogenerate") ?? true;
    if (!isEnabled) {
      return;
    }

    const buildServerJsonPath = path.join(getWorkspacePath(), "buildServer.json");
    const isBuildServerJsonExists = await isFileExists(buildServerJsonPath);
    if (!isBuildServerJsonExists) {
      return;
    }

    const isServerInstalled = await getIsXcodeBuildServerInstalled();
    if (!isServerInstalled) {
      return;
    }

    const xcworkspace = await askXcodeWorkspacePath(this.context);
    await generateBuildServerConfig({
      xcworkspace: xcworkspace,
      scheme: options.scheme,
    });
    await restartSwiftLSP();

    const isShown = this.context.getWorkspaceState("build.xcodeBuildServerAutogenreateInfoShown") ?? false;
    if (!isShown) {
      this.context.updateWorkspaceState("build.xcodeBuildServerAutogenreateInfoShown", true);
      vscode.window.showInformationMessage(`
          INFO: "buildServer.json" file is automatically regenerated every time you change the scheme.
          If you want to disable this feature, you can do it in the settings. This message is shown only once.
      `);
    }
  }

  // Bazel target management
  getSelectedBazelTargetData(): SelectedBazelTargetData | undefined {
    try {
      const storedData = this.context.getWorkspaceState("bazel.selectedTarget");
      if (!storedData) {
        return undefined;
      }
      
      // If it's a string, parse it back to object
      if (typeof storedData === 'string') {
        return JSON.parse(storedData) as SelectedBazelTargetData;
      }
      
      // If it's already an object, return it (backward compatibility)
      return storedData as SelectedBazelTargetData;
    } catch (error) {
      console.error("Failed to get selected Bazel target data:", error);
      return undefined;
    }
  }

  getSelectedBazelTarget(): BazelTreeItem | undefined {
    const selectedTargetData = this.getSelectedBazelTargetData();
    if (!selectedTargetData) {
      return undefined;
    }

    // Create a mock BazelTreeItem from cached data
    return {
      target: {
        name: selectedTargetData.targetName,
        type: selectedTargetData.targetType,
        buildLabel: selectedTargetData.buildLabel,
        testLabel: selectedTargetData.testLabel,
        deps: [],
      },
      package: {
        name: selectedTargetData.packageName,
        path: selectedTargetData.packagePath,
        targets: [],
      },
      workspacePath: selectedTargetData.workspacePath,
    } as any; // Mock BazelTreeItem
  }

  setSelectedBazelTarget(bazelItem: any): void { // BazelTreeItem type
    if (!bazelItem || !bazelItem.target || !bazelItem.package) {
      this.clearSelectedBazelTarget();
      return;
    }

    try {
      // Convert BazelTreeItem to serializable data - avoid any circular references
      const targetType = bazelItem.target.type || 'library';
      const validTargetType: "library" | "test" | "binary" = 
        targetType === 'test' || targetType === 'binary' ? targetType : 'library';
      
      const targetData: SelectedBazelTargetData = {
        targetName: String(bazelItem.target.name || ''),
        targetType: validTargetType,
        buildLabel: String(bazelItem.target.buildLabel || ''),
        testLabel: bazelItem.target.testLabel ? String(bazelItem.target.testLabel) : undefined,
        packageName: String(bazelItem.package.name || ''),
        packagePath: String(bazelItem.package.path || ''),
        workspacePath: String(bazelItem.workspacePath || ''),
      };

      // Use a simple string-based storage to avoid circular references
      this.context.updateWorkspaceState("bazel.selectedTarget", JSON.stringify(targetData));
      this.emitter.emit("selectedBazelTargetUpdated", targetData);
    } catch (error) {
      console.error("❌ Failed to store Bazel target:", error);
      this.clearSelectedBazelTarget();
    }
  }

  clearSelectedBazelTarget(): void {
    this.context.updateWorkspaceState("bazel.selectedTarget", undefined); // Clear the selected target
    this.emitter.emit("selectedBazelTargetUpdated", undefined);
  }
}
