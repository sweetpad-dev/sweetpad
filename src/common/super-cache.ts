import * as path from "node:path";
import type { ExtensionContext } from "./commands";
import type { XcodeScheme } from "./cli/scripts";
import type { BazelPackageInfo, BazelTarget, BazelScheme, BazelXcodeConfiguration } from "../build/bazel/types";
import { commonLogger } from "./logger";

/**
 * Super Cache System for SweetPad
 *
 * Caches all workspace data and never refreshes automatically.
 * Only clears when explicitly requested via "Clear workspace cache" command.
 */

export interface WorkspaceData {
  path: string;
  name: string;
  type: "xcworkspace" | "xcodeproj" | "spm";
  lastScanned?: number;
  schemes: XcodeScheme[];
  configurations: string[];
}

export interface BazelWorkspaceData {
  workspacePath: string;
  packages: BazelPackageInfo[];
  allTargets: BazelTarget[];
  allTestTargets: BazelTarget[];
  allSchemes: BazelScheme[];
  allConfigurations: BazelXcodeConfiguration[];
  lastScanned?: number;
}

export interface SuperCacheData {
  // Xcode/SPM workspaces
  workspaces: Record<string, WorkspaceData>;

  // Bazel workspaces
  bazelWorkspaces: Record<string, BazelWorkspaceData>;

  // Discovery cache
  discoveredWorkspacePaths: string[];
  discoveredBazelPaths: string[];

  // Cache metadata
  version: string;
  createdAt: number;
  lastCleared?: number;
}

class SuperCacheManager {
  private data: SuperCacheData;
  private context?: ExtensionContext;
  private readonly CACHE_VERSION = "1.0.0";

  constructor() {
    this.data = this.initializeEmptyCache();
  }

  private initializeEmptyCache(): SuperCacheData {
    return {
      workspaces: {},
      bazelWorkspaces: {},
      discoveredWorkspacePaths: [],
      discoveredBazelPaths: [],
      version: this.CACHE_VERSION,
      createdAt: Date.now(),
    };
  }

  async setContext(context: ExtensionContext): Promise<void> {
    this.context = context;
    await this.loadFromStorage();
  }

  private async loadFromStorage(): Promise<void> {
    if (!this.context) return;

    try {
      commonLogger.log("🔄 Loading super cache from storage...");
      const storedData = this.context.getWorkspaceState("superCacheData" as any) as any;

      if (storedData && typeof storedData === "object") {
        // Validate stored data structure and version
        if ((storedData as any).version === this.CACHE_VERSION) {
          this.data = storedData as SuperCacheData;
          const workspaceCount = Object.keys(this.data.workspaces).length;
          const bazelCount = Object.keys(this.data.bazelWorkspaces).length;
          const discoveredPaths = this.data.discoveredWorkspacePaths.length;

          commonLogger.log(
            `✅ Super cache loaded successfully: ${workspaceCount} workspaces, ${bazelCount} bazel workspaces, ${discoveredPaths} discovered paths`,
          );

          // Log some cache contents for debugging
          if (workspaceCount > 0) {
            const workspacePaths = Object.keys(this.data.workspaces);
            commonLogger.log(
              `📂 Cached workspaces: ${workspacePaths.slice(0, 3).join(", ")}${workspaceCount > 3 ? ` +${workspaceCount - 3} more` : ""}`,
            );
          }
        } else {
          const storedVersion = (storedData as any).version || "unknown";
          commonLogger.log(
            `🔄 Super cache version mismatch (stored: ${storedVersion}, expected: ${this.CACHE_VERSION}), initializing fresh cache`,
          );
          this.data = this.initializeEmptyCache();
        }
      } else {
        commonLogger.log("📭 No existing super cache found, initializing fresh cache");
        this.data = this.initializeEmptyCache();
      }
    } catch (error) {
      commonLogger.error("❌ Failed to load super cache from storage", { error });
      this.data = this.initializeEmptyCache();
    }
  }

  private async saveToStorage(): Promise<void> {
    if (!this.context) return;

    try {
      await this.context.updateWorkspaceState("superCacheData" as any, this.data);
    } catch (error) {
      commonLogger.error("Failed to save super cache to storage", { error });
    }
  }

  // === WORKSPACE CACHE METHODS ===

  /**
   * Get cached workspace data by path
   */
  getWorkspace(workspacePath: string): WorkspaceData | undefined {
    return this.data.workspaces[workspacePath];
  }

  /**
   * Get all cached workspaces
   */
  getAllWorkspaces(): WorkspaceData[] {
    return Object.values(this.data.workspaces);
  }

  /**
   * Cache workspace data
   */
  async cacheWorkspace(workspaceData: WorkspaceData): Promise<void> {
    this.data.workspaces[workspaceData.path] = {
      ...workspaceData,
      lastScanned: Date.now(),
    };
    await this.saveToStorage();
    commonLogger.log(`Cached workspace: ${workspaceData.name} (${workspaceData.schemes.length} schemes)`);
  }

  /**
   * Get cached schemes for a workspace
   */
  getWorkspaceSchemes(workspacePath: string): XcodeScheme[] {
    const workspace = this.data.workspaces[workspacePath];
    return workspace?.schemes || [];
  }

  /**
   * Get cached configurations for a workspace
   */
  getWorkspaceConfigurations(workspacePath: string): string[] {
    const workspace = this.data.workspaces[workspacePath];
    return workspace?.configurations || [];
  }

  // === BAZEL CACHE METHODS ===

  /**
   * Get cached Bazel workspace data
   */
  getBazelWorkspace(workspacePath: string): BazelWorkspaceData | undefined {
    return this.data.bazelWorkspaces[workspacePath];
  }

  /**
   * Get all cached Bazel workspaces
   */
  getAllBazelWorkspaces(): BazelWorkspaceData[] {
    return Object.values(this.data.bazelWorkspaces);
  }

  /**
   * Cache Bazel workspace data
   */
  async cacheBazelWorkspace(workspacePath: string, packages: BazelPackageInfo[]): Promise<void> {
    // Aggregate all data from packages
    const allTargets: BazelTarget[] = [];
    const allTestTargets: BazelTarget[] = [];
    const allSchemes: BazelScheme[] = [];
    const allConfigurations: BazelXcodeConfiguration[] = [];

    for (const pkg of packages) {
      allTargets.push(...pkg.parseResult.targets);
      allTestTargets.push(...pkg.parseResult.targetsTest);
      allSchemes.push(...pkg.parseResult.xcschemes);
      allConfigurations.push(...pkg.parseResult.xcode_configurations);
    }

    this.data.bazelWorkspaces[workspacePath] = {
      workspacePath,
      packages,
      allTargets,
      allTestTargets,
      allSchemes,
      allConfigurations,
      lastScanned: Date.now(),
    };

    await this.saveToStorage();
    commonLogger.log(
      `Cached Bazel workspace: ${workspacePath} (${packages.length} packages, ${allTargets.length} targets)`,
    );
  }

  /**
   * Get cached Bazel packages for workspace
   */
  getBazelPackages(workspacePath: string): BazelPackageInfo[] {
    const workspace = this.data.bazelWorkspaces[workspacePath];
    return workspace?.packages || [];
  }

  /**
   * Get cached Bazel targets for workspace
   */
  getBazelTargets(workspacePath: string): BazelTarget[] {
    const workspace = this.data.bazelWorkspaces[workspacePath];
    return workspace?.allTargets || [];
  }

  /**
   * Get cached Bazel test targets for workspace
   */
  getBazelTestTargets(workspacePath: string): BazelTarget[] {
    const workspace = this.data.bazelWorkspaces[workspacePath];
    return workspace?.allTestTargets || [];
  }

  /**
   * Get cached Bazel schemes for workspace
   */
  getBazelSchemes(workspacePath: string): BazelScheme[] {
    const workspace = this.data.bazelWorkspaces[workspacePath];
    return workspace?.allSchemes || [];
  }

  /**
   * Get cached Bazel configurations for workspace
   */
  getBazelConfigurations(workspacePath: string): BazelXcodeConfiguration[] {
    const workspace = this.data.bazelWorkspaces[workspacePath];
    return workspace?.allConfigurations || [];
  }

  // === DISCOVERY CACHE METHODS ===

  /**
   * Get cached discovered workspace paths
   */
  getDiscoveredWorkspacePaths(): string[] {
    return [...this.data.discoveredWorkspacePaths];
  }

  /**
   * Cache discovered workspace paths
   */
  async cacheDiscoveredWorkspacePaths(paths: string[]): Promise<void> {
    this.data.discoveredWorkspacePaths = [...new Set(paths)]; // Remove duplicates
    await this.saveToStorage();
    commonLogger.log(`Cached ${paths.length} discovered workspace paths`);
  }

  /**
   * Get cached discovered Bazel paths
   */
  getDiscoveredBazelPaths(): string[] {
    return [...this.data.discoveredBazelPaths];
  }

  /**
   * Cache discovered Bazel paths
   */
  async cacheDiscoveredBazelPaths(paths: string[]): Promise<void> {
    this.data.discoveredBazelPaths = [...new Set(paths)]; // Remove duplicates
    await this.saveToStorage();
    commonLogger.log(`Cached ${paths.length} discovered Bazel paths`);
  }

  // === UTILITY METHODS ===

  /**
   * Check if workspace is cached
   */
  isWorkspaceCached(workspacePath: string): boolean {
    return workspacePath in this.data.workspaces;
  }

  /**
   * Check if Bazel workspace is cached
   */
  isBazelWorkspaceCached(workspacePath: string): boolean {
    return workspacePath in this.data.bazelWorkspaces;
  }

  /**
   * Get cache statistics
   */
  getCacheStats(): {
    workspaceCount: number;
    bazelWorkspaceCount: number;
    totalSchemes: number;
    totalBazelTargets: number;
    totalBazelSchemes: number;
    cacheSize: string;
    createdAt: Date;
    lastCleared?: Date;
  } {
    const workspaces = Object.values(this.data.workspaces);
    const bazelWorkspaces = Object.values(this.data.bazelWorkspaces);

    const totalSchemes = workspaces.reduce((sum, ws) => sum + ws.schemes.length, 0);
    const totalBazelTargets = bazelWorkspaces.reduce((sum, bws) => sum + bws.allTargets.length, 0);
    const totalBazelSchemes = bazelWorkspaces.reduce((sum, bws) => sum + bws.allSchemes.length, 0);

    // Estimate cache size
    const cacheString = JSON.stringify(this.data);
    const sizeInBytes = new TextEncoder().encode(cacheString).length;
    const sizeInKB = Math.round(sizeInBytes / 1024);
    const cacheSize = sizeInKB < 1024 ? `${sizeInKB} KB` : `${(sizeInKB / 1024).toFixed(1)} MB`;

    return {
      workspaceCount: workspaces.length,
      bazelWorkspaceCount: bazelWorkspaces.length,
      totalSchemes,
      totalBazelTargets,
      totalBazelSchemes,
      cacheSize,
      createdAt: new Date(this.data.createdAt),
      lastCleared: this.data.lastCleared ? new Date(this.data.lastCleared) : undefined,
    };
  }

  /**
   * Clear all cached data (only when explicitly requested)
   */
  async clearCache(): Promise<void> {
    const stats = this.getCacheStats();
    this.data = this.initializeEmptyCache();
    this.data.lastCleared = Date.now();

    await this.saveToStorage();

    commonLogger.log("Super cache cleared", {
      previousStats: stats,
    });
  }

  /**
   * Clear specific workspace from cache
   */
  async clearWorkspace(workspacePath: string): Promise<void> {
    if (workspacePath in this.data.workspaces) {
      delete this.data.workspaces[workspacePath];
      await this.saveToStorage();
      commonLogger.log(`Cleared workspace from cache: ${workspacePath}`);
    }
  }

  /**
   * Clear specific Bazel workspace from cache
   */
  async clearBazelWorkspace(workspacePath: string): Promise<void> {
    if (workspacePath in this.data.bazelWorkspaces) {
      delete this.data.bazelWorkspaces[workspacePath];
      await this.saveToStorage();
      commonLogger.log(`Cleared Bazel workspace from cache: ${workspacePath}`);
    }
  }

  /**
   * Export cache data for debugging
   */
  exportCacheData(): SuperCacheData {
    return JSON.parse(JSON.stringify(this.data)); // Deep copy
  }
}

// Singleton instance
export const superCache = new SuperCacheManager();
