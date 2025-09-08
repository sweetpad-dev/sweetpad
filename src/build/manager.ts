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
import { superCache } from "../common/super-cache";

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
    // Initialize super cache with context (async but not awaited here for compatibility)
    void superCache.setContext(context);
  }

  // New async method to properly initialize with cache loading
  async initializeWithContext(context: ExtensionContext): Promise<void> {
    this._context = context;
    // Initialize super cache with context and wait for it to load
    await superCache.setContext(context);
  }

  get context(): ExtensionContext {
    if (!this._context) {
      throw new Error("Context is not set");
    }
    return this._context;
  }

  async getSchemas(options?: { refresh?: boolean }): Promise<XcodeScheme[]> {
    const xcworkspace = getCurrentXcodeWorkspacePath(this.context);

    // If refresh is forced, skip cache and refresh
    if (options?.refresh) {
      commonLogger.log("🔄 Refresh forced, skipping cache");
      return await this.refresh();
    }

    // Try to get from super cache first
    if (xcworkspace) {
      const cachedSchemes = superCache.getWorkspaceSchemes(xcworkspace);
      if (cachedSchemes.length > 0) {
        commonLogger.log(`📦 Using cached schemes for ${xcworkspace}: ${cachedSchemes.length} schemes`);
        return cachedSchemes;
      } else {
        commonLogger.log(`📭 No cached schemes found for ${xcworkspace}, will refresh`);
      }
    } else {
      commonLogger.log("📭 No workspace path available, will refresh");
    }

    // If not cached, refresh and cache
    return await this.refresh();
  }

  async refresh(): Promise<XcodeScheme[]> {
    // Always get the latest workspace path from context
    const xcworkspace = getCurrentXcodeWorkspacePath(this.context);

    if (!xcworkspace) {
      commonLogger.warn("No workspace path available for refresh");
      return [];
    }

    try {
      commonLogger.log(`Refreshing schemes for workspace: ${xcworkspace}`);

      const schemes = await getSchemes({
        xcworkspace: xcworkspace,
      });

      // Cache the workspace data in super cache
      const workspaceName = path.basename(xcworkspace);
      const workspaceType = xcworkspace.endsWith(".xcworkspace")
        ? "xcworkspace"
        : xcworkspace.endsWith(".xcodeproj")
          ? "xcodeproj"
          : "spm";

      await superCache.cacheWorkspace({
        path: xcworkspace,
        name: workspaceName,
        type: workspaceType,
        schemes,
        configurations: [], // TODO: Add configuration discovery later
      });

      this.emitter.emit("updated");
      return schemes;
    } catch (error) {
      // If there's an error getting schemes, return empty array
      commonLogger.error("Failed to get schemes", { error });
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

    // Clear any selected Bazel target when workspace changes
    this.clearSelectedBazelTarget();

    this.context.updateWorkspaceState("build.xcodeWorkspacePath", workspacePath);
    this.emitter.emit("currentWorkspacePathUpdated", workspacePath);

    // Allow skipping the automatic refresh when needed
    if (!skipRefresh) {
      // Use getSchemas instead of refresh to check cache first
      void this.getSchemas();
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
    // Cache is now managed by superCache, this method is kept for compatibility
    // The cache will only be cleared via the "Clear workspace cache" command
    commonLogger.log("clearSchemesCache called - cache is now managed by superCache");
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
      if (typeof storedData === "string") {
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

  setSelectedBazelTarget(bazelItem: any): void {
    // BazelTreeItem type
    if (!bazelItem || !bazelItem.target || !bazelItem.package) {
      this.clearSelectedBazelTarget();
      return;
    }

    try {
      // Convert BazelTreeItem to serializable data - avoid any circular references
      const targetType = bazelItem.target.type || "library";
      const validTargetType: "library" | "test" | "binary" =
        targetType === "test" || targetType === "binary" ? targetType : "library";

      const targetData: SelectedBazelTargetData = {
        targetName: String(bazelItem.target.name || ""),
        targetType: validTargetType,
        buildLabel: String(bazelItem.target.buildLabel || ""),
        testLabel: bazelItem.target.testLabel ? String(bazelItem.target.testLabel) : undefined,
        packageName: String(bazelItem.package.name || ""),
        packagePath: String(bazelItem.package.path || ""),
        workspacePath: String(bazelItem.workspacePath || ""),
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
