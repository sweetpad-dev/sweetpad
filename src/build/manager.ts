import events from "node:events";
import * as path from "node:path";
import * as vscode from "vscode";
import {
  type XcodeScheme,
  generateBuildServerConfig,
  getBasicProjectInfo,
  getIsXcodeBuildServerInstalled,
  getSchemes,
} from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { isFileExists } from "../common/files";
import { askXcodeWorkspacePath, getCurrentXcodeWorkspacePath, getWorkspacePath, restartSwiftLSP } from "./utils";

type IEventMap = {
  refreshStarted: [];
  updated: [];
  refreshError: [error: Error];
  defaultSchemeForBuildUpdated: [scheme: string | undefined];
  defaultSchemeForTestingUpdated: [scheme: string | undefined];
};
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

  async refresh(): Promise<XcodeScheme[]> {
    this.emitter.emit("refreshStarted");
    try {
      getBasicProjectInfo.clearCache();
      
      const xcworkspace = getCurrentXcodeWorkspacePath(this.context);

      const scheme = await getSchemes({
        xcworkspace: xcworkspace,
      });

      this.cache = scheme;
      
      await this.validateDefaultSchemes();
      
      this.emitter.emit("updated");
      return this.cache;
    } catch (error) {
      this.emitter.emit("refreshError", error as Error);
      throw error;
    }
  }

  async getSchemas(options?: { refresh?: boolean }): Promise<XcodeScheme[]> {
    if (this.cache === undefined || options?.refresh) {
      return await this.refresh();
    }
    return this.cache;
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

  /**
   * Validates that the current default schemes still exist in the refreshed schemes list.
   * If a default scheme no longer exists, it will be cleared.
   */
  private async validateDefaultSchemes(): Promise<void> {
    if (!this.cache) {
      return;
    }

    const schemeNames = this.cache.map(scheme => scheme.name);
    const currentBuildScheme = this.getDefaultSchemeForBuild();
    if (currentBuildScheme && !schemeNames.includes(currentBuildScheme)) {
      this.setDefaultSchemeForBuild(undefined);
    }
    
    const currentTestingScheme = this.getDefaultSchemeForTesting();
    if (currentTestingScheme && !schemeNames.includes(currentTestingScheme)) {
      this.setDefaultSchemeForTesting(undefined);
    }
  }
}
