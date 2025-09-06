import path from "node:path";
import * as vscode from "vscode";
import * as fs from "node:fs/promises";
import type { Dirent } from "node:fs";
import type { XcodeScheme } from "../../common/cli/scripts";
import { getSchemes } from "../../common/cli/scripts";
import type { ExtensionContext } from "../../common/commands";
import { commonLogger } from "../../common/logger";
import type { BuildManager } from "../manager";
import { getCurrentXcodeWorkspacePath, getWorkspacePath } from "../utils";
import {
  detectXcodeWorkspacesPaths,
  parseBazelBuildFile,
  findBazelWorkspaceRoot,
  type BazelTarget,
  type BazelPackage,
} from "../utils";
import type { SelectedBazelTargetData } from "../manager";
import { ExtensionError } from "../../common/errors";
import type { WorkspaceEventData, WorkspaceCacheData } from "./types";
import { WorkspaceGroupTreeItem, type IWorkspaceTreeProvider } from "./items/workspace-group-tree-item";
import { WorkspaceSectionTreeItem } from "./items/workspace-section-tree-item";
import { BuildTreeItem, type IBuildTreeProvider } from "./items/build-tree-item";
import { BazelTreeItem, type IBazelTreeProvider } from "./items/bazel-tree-item";

export class WorkspaceTreeProvider
  implements vscode.TreeDataProvider<vscode.TreeItem>, IWorkspaceTreeProvider, IBuildTreeProvider, IBazelTreeProvider
{
  private _onDidChangeTreeData = new vscode.EventEmitter<WorkspaceEventData>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;
  public context: ExtensionContext | undefined;
  public buildManager: BuildManager;

  public defaultWorkspacePath: string | undefined;
  public defaultSchemeForBuild: string | undefined;
  public defaultSchemeForTesting: string | undefined;
  private workspaces: WorkspaceGroupTreeItem[] = [];
  private recentWorkspaces: WorkspaceGroupTreeItem[] = [];
  private isLoadingWorkspaces: boolean = false;
  private recentWorkspacesStorage: string[] = [];
  private loadingItems = new Map<string, boolean>();
  private loadingItem: WorkspaceGroupTreeItem | null = null;
  private searchTerm: string = "";
  private isSearchActive: boolean = false;

  // Cached filtered data to avoid recomputation
  private cachedFilteredWorkspaces: WorkspaceGroupTreeItem[] | null = null;
  private cachedFilteredRecentWorkspaces: WorkspaceGroupTreeItem[] | null = null;
  private cachedSchemesForWorkspaces = new Map<string, BuildTreeItem[]>();

  // Track last computed search term to avoid recomputation
  private lastComputedSearchTerm: string = "";

  // Cache individual parsed BUILD.bazel files to avoid re-parsing on each click
  private cachedBazelFiles = new Map<string, BazelPackage | null>();
  private bazelFileCacheTimestamps = new Map<string, number>();

  // Cache of currently displayed Bazel targets for command lookup
  public currentBazelTargets = new Map<string, BazelTreeItem>();

  // Persistent workspace cache for instant loading
  private persistentCacheKey = "sweetpad.workspaces.cache";
  private persistentCacheVersion = "1.1.0"; // Bumped version to invalidate old large caches

  // Limits to prevent performance issues
  private readonly MAX_WORKSPACES = 1000; // Reasonable limit
  private readonly MAX_RECENT_WORKSPACES = 3;

  // Track loading state for each section type
  private sectionsLoading = new Set<string>(["workspace", "package", "bazel"]);

  // Track individual searches within sections (for workspace section which has multiple searches)
  private subsectionLoadingCount = new Map<string, number>([
    ["workspace", 2], // project.xcworkspace search (counts as 2 for compatibility)
    ["package", 1], // Package.swift search
    ["bazel", 1], // BUILD.bazel search
  ]);

  // Track if workspaces need sorting
  private workspacesSorted = true;

  // Bazel status bar reference for updates
  private bazelStatusBar: any = null; // BazelTargetStatusBar reference

  // Throttled UI refresh for better performance during bulk loading
  private refreshThrottleTimer: NodeJS.Timeout | null = null;

  // Throttled cache save to avoid excessive disk writes
  private cacheSaveTimer: NodeJS.Timeout | null = null;

  constructor(options: { context: ExtensionContext; buildManager: BuildManager }) {
    this.context = options.context;
    this.buildManager = options.buildManager;
    this.defaultWorkspacePath = getCurrentXcodeWorkspacePath(this.context);

    this.buildManager.on("updated", () => {
      this.refresh();
    });
    this.buildManager.on("currentWorkspacePathUpdated", (workspacePath) => {
      this.defaultWorkspacePath = workspacePath;
      if (workspacePath) {
        this.addToRecentWorkspaces(workspacePath);
      }
      this.buildManager.clearSchemesCache();
      this.refresh();
    });
    this.buildManager.on("defaultSchemeForBuildUpdated", (scheme) => {
      this.defaultSchemeForBuild = scheme;
      this.refresh();
    });
    this.buildManager.on("defaultSchemeForTestingUpdated", (scheme) => {
      this.defaultSchemeForTesting = scheme;
      this.refresh();
    });
    this.defaultSchemeForBuild = this.buildManager.getDefaultSchemeForBuild();
    this.defaultSchemeForTesting = this.buildManager.getDefaultSchemeForTesting();

    // Initialize context for UI elements
    this.updateSearchContext();

    // Initial workspace loading - load cache immediately, then refresh in background
    this.initializeWorkspaces();
  }

  // Public method to refresh a specific tree item
  refreshTreeItem(item: WorkspaceGroupTreeItem | null): void {
    // Only refresh the specific item, not the entire tree
    this._onDidChangeTreeData.fire(item);
  }

  // Cache invalidation methods - selective to preserve performance
  private invalidateFilterCache(): void {
    this.cachedFilteredWorkspaces = null;
    this.cachedFilteredRecentWorkspaces = null;
    this.cachedSchemesForWorkspaces.clear();
    this.lastComputedSearchTerm = ""; // Reset so filter gets recomputed
    // Note: Don't clear workspaceMetadataCache here - it can persist across workspace list changes
  }

  private invalidateDataCache(): void {
    // Called when underlying data changes
    this.invalidateFilterCache();
    // Also invalidate Bazel file cache when workspace data changes
    this.cachedBazelFiles.clear();
    this.bazelFileCacheTimestamps.clear();

    // Clear current Bazel targets cache
    this.currentBazelTargets.clear();

    // Clear workspace metadata cache to ensure consistency
    this.workspaceMetadataCache.clear();
  }

  // Debounced search functionality with caching
  private searchDebounceTimer: NodeJS.Timeout | null = null;

  public setSearchTerm(searchTerm: string): void {
    const previousSearchTerm = this.searchTerm;
    const wasSearchActive = this.isSearchActive;
    this.searchTerm = searchTerm.toLowerCase();
    this.isSearchActive = searchTerm.length > 0;

    // Update context for conditional UI elements immediately
    this.updateSearchContext();

    // Clear existing debounce timer
    if (this.searchDebounceTimer) {
      clearTimeout(this.searchDebounceTimer);
    }

    // Only recompute filtered cache if search term actually changed
    if (previousSearchTerm !== this.searchTerm) {
      // Fast path: If clearing search (empty term), update immediately
      if (!this.searchTerm || this.searchTerm.length === 0) {
        this.computeFilteredCache();
        this._onDidChangeTreeData.fire(null);
        return;
      }

      // If this is the start of a new search, preload content for better results
      if (!wasSearchActive && this.isSearchActive) {
        this.preloadContentForSearch();
      }

      // For instant feedback, show search UI changes immediately
      this._onDidChangeTreeData.fire(null);

      // Then debounce the actual filtering work for non-empty searches
      this.searchDebounceTimer = setTimeout(() => {
        const start = performance.now();
        this.computeFilteredCache();
        const end = performance.now();

        // Log performance only for slow operations
        if (end - start > 200) {
          console.warn(`Slow filter: ${Math.round(end - start)}ms`);
        }

        // Update UI with filtered results
        this._onDidChangeTreeData.fire(null);
        this.searchDebounceTimer = null;
      }, 50); // Reduced to 50ms for better responsiveness
    }
  }

  public clearSearch(): void {
    if (this.searchTerm !== "" || this.isSearchActive) {
      // Clear debounce timer if active
      if (this.searchDebounceTimer) {
        clearTimeout(this.searchDebounceTimer);
        this.searchDebounceTimer = null;
      }

      this.searchTerm = "";
      this.isSearchActive = false;

      // Fast path: Clear filter cache immediately - no computation needed
      this.cachedFilteredWorkspaces = null;
      this.cachedFilteredRecentWorkspaces = null;
      this.lastComputedSearchTerm = "";

      // Update context for conditional UI elements (async to not block)
      this.updateSearchContext();

      // Use immediate update when clearing search - no debounce needed
      this._onDidChangeTreeData.fire(null);
    }
  }

  private updateSearchContext(): void {
    // Set context variables for conditional UI elements (completely async to not block)
    Promise.all([
      vscode.commands.executeCommand("setContext", "sweetpad.builds.searchActive", this.isSearchActive),
      vscode.commands.executeCommand("setContext", "sweetpad.builds.searchTerm", this.searchTerm || ""),
    ]).catch((error) => console.warn("Failed to update search context:", error));
  }

  private computeFilteredCache(): void {
    // Fast path: If no search active, just clear cache without computation
    if (!this.isSearchActive || !this.searchTerm) {
      this.cachedFilteredWorkspaces = null;
      this.cachedFilteredRecentWorkspaces = null;
      this.lastComputedSearchTerm = "";
      return;
    }

    // Skip recomputation if search term hasn't changed
    if (
      this.searchTerm === this.lastComputedSearchTerm &&
      this.cachedFilteredWorkspaces !== null &&
      this.cachedFilteredRecentWorkspaces !== null
    ) {
      return; // Cache is still valid
    }

    // Performance timing for optimization
    const start = performance.now();

    // Early exit if no workspaces to filter
    if (this.workspaces.length === 0 && this.recentWorkspaces.length === 0) {
      this.cachedFilteredWorkspaces = [];
      this.cachedFilteredRecentWorkspaces = [];
      this.lastComputedSearchTerm = this.searchTerm;
      return;
    }

    // Cache filtered workspaces - use highly optimized filtering
    this.cachedFilteredWorkspaces = this.workspaces.length > 0 ? this.filterWorkspacesOptimized(this.workspaces) : [];
    this.cachedFilteredRecentWorkspaces =
      this.recentWorkspaces.length > 0 ? this.filterWorkspacesOptimized(this.recentWorkspaces) : [];

    // Mark this search term as computed
    this.lastComputedSearchTerm = this.searchTerm;

    const end = performance.now();
    const duration = Math.round(end - start);

    // Only log slow operations to reduce noise
    if (duration > 100) {
      console.warn(`Slow filtering: ${duration}ms for ${this.workspaces.length} workspaces`);
    }
  }

  public getSearchTerm(): string {
    return this.searchTerm;
  }

  public isSearching(): boolean {
    return this.isSearchActive;
  }

  // Selected Bazel target management - delegate to BuildManager
  public getSelectedBazelTarget(): BazelTreeItem | null {
    // We can't return the full BazelTreeItem since it's not stored
    // This method is mainly for compatibility with icon checking
    return null; // Will be determined by comparing build labels
  }

  public getSelectedBazelTargetData(): SelectedBazelTargetData | undefined {
    return this.buildManager.getSelectedBazelTargetData();
  }

  public setSelectedBazelTarget(target: BazelTreeItem | null): void {
    // Update BuildManager (primary source of truth)
    this.buildManager.setSelectedBazelTarget(target);

    // Refresh the tree to update selection indicators
    this._onDidChangeTreeData.fire(null);

    // Update context for command availability
    const targetData = this.buildManager.getSelectedBazelTargetData();
    void vscode.commands.executeCommand("setContext", "sweetpad.bazel.hasSelectedTarget", !!targetData);
    void vscode.commands.executeCommand(
      "setContext",
      "sweetpad.bazel.selectedTargetType",
      targetData?.targetType || null,
    );
  }

  public clearSelectedBazelTarget(): void {
    this.setSelectedBazelTarget(null);
  }

  public setBazelStatusBar(statusBar: any): void {
    this.bazelStatusBar = statusBar;

    // Listen for BuildManager events to update status bar
    this.buildManager.on("selectedBazelTargetUpdated", () => {
      if (this.bazelStatusBar) {
        this.bazelStatusBar.update();
      }
    });
  }

  // Get a cached Bazel target by build label
  public getBazelTargetByLabel(buildLabel: string): BazelTreeItem | null {
    return this.currentBazelTargets.get(buildLabel) || null;
  }

  // Filter workspaces based on search term - optimized version
  private filterWorkspaces(workspaces: WorkspaceGroupTreeItem[]): WorkspaceGroupTreeItem[] {
    if (!this.isSearchActive || !this.searchTerm) {
      return workspaces;
    }
    return this.filterWorkspacesOptimized(workspaces);
  }

  // Enhanced workspace filtering that checks both workspace names and their content (schemes/targets)
  private filterWorkspacesOptimized(workspaces: WorkspaceGroupTreeItem[]): WorkspaceGroupTreeItem[] {
    if (!this.isSearchActive || !this.searchTerm) {
      return workspaces;
    }

    const searchTerm = this.searchTerm;
    const filteredWorkspaces: WorkspaceGroupTreeItem[] = [];

    // Pre-allocate array for better performance
    const len = workspaces.length;

    for (let i = 0; i < len; i++) {
      const workspace = workspaces[i];
      let shouldInclude = false;

      // Quick label check first (most common case)
      const label = workspace.label;
      if (label && typeof label === "string") {
        // Use indexOf for better performance than includes
        if (label.toLowerCase().indexOf(searchTerm) !== -1) {
          shouldInclude = true;
        }
      }

      // Check workspace path only if label didn't match
      if (!shouldInclude) {
        const workspacePath = workspace.workspacePath;
        if (workspacePath.toLowerCase().indexOf(searchTerm) !== -1) {
          shouldInclude = true;
        }
      }

      // If workspace name doesn't match, check if it contains matching schemes/targets
      if (!shouldInclude) {
        shouldInclude = this.workspaceContainsMatchingContent(workspace);
      }

      if (shouldInclude) {
        filteredWorkspaces.push(workspace);
      }
    }

    return filteredWorkspaces;
  }

  // Check if workspace contains schemes/targets matching the search term (synchronous)
  private workspaceContainsMatchingContent(workspace: WorkspaceGroupTreeItem): boolean {
    if (!this.isSearchActive || !this.searchTerm) {
      return false;
    }

    const searchTerm = this.searchTerm;

    // Check if this is a Bazel workspace
    const isBazelWorkspace =
      workspace.workspacePath.endsWith("BUILD.bazel") || workspace.workspacePath.endsWith("BUILD");

    if (isBazelWorkspace) {
      // For Bazel workspaces, check cached targets
      const bazelPackage = this.cachedBazelFiles.get(workspace.workspacePath);
      if (bazelPackage && bazelPackage.targets) {
        return bazelPackage.targets.some(
          (target) =>
            target.name.toLowerCase().indexOf(searchTerm) !== -1 ||
            (target.buildLabel && target.buildLabel.toLowerCase().indexOf(searchTerm) !== -1),
        );
      }
    } else {
      // For Xcode/SPM workspaces, check cached schemes
      // Try different cache keys that might exist
      const cacheKeys = [
        `${workspace.workspacePath}:${searchTerm}`,
        `${workspace.workspacePath}:`,
        workspace.workspacePath,
      ];

      for (const cacheKey of cacheKeys) {
        const cachedSchemes = this.cachedSchemesForWorkspaces.get(cacheKey);
        if (cachedSchemes) {
          // Check if any scheme name contains the search term
          return cachedSchemes.some((scheme) => {
            const schemeName = scheme.label;
            return schemeName && typeof schemeName === "string" && schemeName.toLowerCase().indexOf(searchTerm) !== -1;
          });
        }
      }
    }

    // If not in cache, we can't determine without async operations
    // For now, return false to keep filtering fast
    // The content will be checked when the workspace is expanded
    return false;
  }

  // Preload content for workspaces when search starts for better search results
  private preloadContentForSearch(): void {
    // Run preloading in background without blocking UI
    setTimeout(() => {
      this.preloadContentForSearchAsync();
    }, 0);
  }

  private async preloadContentForSearchAsync(): Promise<void> {
    // Only preload a limited number of workspaces to avoid performance issues
    const workspacesToPreload = [...this.recentWorkspaces, ...this.workspaces.slice(0, 10)];
    const promises: Promise<void>[] = [];

    for (const workspace of workspacesToPreload) {
      // Skip if already cached
      const cacheKey = `${workspace.workspacePath}:`;
      if (this.cachedSchemesForWorkspaces.has(cacheKey)) {
        continue;
      }

      // Check if this is a Bazel workspace
      const isBazelWorkspace =
        workspace.workspacePath.endsWith("BUILD.bazel") || workspace.workspacePath.endsWith("BUILD");

      if (isBazelWorkspace) {
        // For Bazel workspaces, preload targets if not cached
        if (!this.cachedBazelFiles.has(workspace.workspacePath)) {
          promises.push(
            this.getCachedBazelPackage(workspace.workspacePath)
              .then(() => {
                // Target data is now cached
              })
              .catch(() => {
                // Ignore errors during preloading
              }),
          );
        }
      } else {
        // For Xcode/SPM workspaces, preload schemes
        promises.push(
          this.getSchemesDirectly(workspace.workspacePath)
            .then((schemes) => {
              // Cache the schemes with empty search term for future use
              this.cachedSchemesForWorkspaces.set(`${workspace.workspacePath}:`, schemes);
            })
            .catch(() => {
              // Ignore errors during preloading
            }),
        );
      }

      // Limit concurrent operations to avoid overwhelming the system
      if (promises.length >= 3) {
        await Promise.all(promises);
        promises.length = 0; // Clear the array
      }
    }

    // Wait for any remaining promises
    if (promises.length > 0) {
      await Promise.all(promises);
    }

    // Refresh search results now that we have more cached content
    if (this.isSearchActive) {
      this.computeFilteredCache();
      this._onDidChangeTreeData.fire(null);
    }
  }

  // Filter schemes based on search term - highly optimized version
  private async filterSchemes(schemes: BuildTreeItem[]): Promise<BuildTreeItem[]> {
    if (!this.isSearchActive || !this.searchTerm) {
      return schemes;
    }

    // Use highly optimized filtering approach
    const searchTerm = this.searchTerm;
    const filteredSchemes: BuildTreeItem[] = [];
    const len = schemes.length;

    // Pre-allocate for better performance
    for (let i = 0; i < len; i++) {
      const scheme = schemes[i];
      const schemeName = scheme.label;

      if (schemeName && typeof schemeName === "string") {
        // Use indexOf for better performance than includes
        if (schemeName.toLowerCase().indexOf(searchTerm) !== -1) {
          filteredSchemes.push(scheme);
        }
      }
    }

    return filteredSchemes;
  }

  private refresh(): void {
    // Clear any loading states before refreshing
    this.clearAllLoadingStates();

    // Invalidate cache since underlying data is changing
    this.invalidateDataCache();

    this._onDidChangeTreeData.fire(null);
    this.getChildren();
  }

  // Add a workspace to recents
  private addToRecentWorkspaces(workspacePath: string): void {
    // Remove if already exists to avoid duplicates
    this.recentWorkspacesStorage = this.recentWorkspacesStorage.filter((path) => path !== workspacePath);

    // Add to the front of the array
    this.recentWorkspacesStorage.unshift(workspacePath);

    // Keep only the most recent workspaces with our new limit
    this.recentWorkspacesStorage = this.recentWorkspacesStorage.slice(0, this.MAX_RECENT_WORKSPACES);

    // Update our instance variable with tree items
    this.recentWorkspaces = this.recentWorkspacesStorage.map((path) => {
      // Determine if this recent item is the current workspace
      const isCurrentWorkspace = path === this.defaultWorkspacePath;

      return new WorkspaceGroupTreeItem({
        workspacePath: path,
        provider: this,
        isRecent: true,
      });
    });

    // Invalidate cache since recent workspaces changed
    this.invalidateFilterCache();

    // Save updated recent workspaces to persistent cache
    this.throttledCacheSave();
  }

  // Throttled UI refresh for better performance during bulk loading
  private throttledRefresh(): void {
    if (this.refreshThrottleTimer) {
      return; // Already scheduled
    }

    this.refreshThrottleTimer = setTimeout(() => {
      this._onDidChangeTreeData.fire(undefined);
      this.refreshThrottleTimer = null;
    }, 100); // 100ms throttle
  }

  // Throttled cache save to avoid excessive disk writes during workspace discovery
  private throttledCacheSave(): void {
    if (this.cacheSaveTimer) {
      return; // Already scheduled
    }

    this.cacheSaveTimer = setTimeout(() => {
      void this.saveToPersistentCache();
      this.cacheSaveTimer = null;
    }, 2000); // Save cache every 2 seconds max during active discovery
  }

  // Check if we should skip this workspace to improve quality and performance
  private shouldSkipWorkspace(workspacePath: string): boolean {
    const workspaceRoot = getWorkspacePath();
    const relativePath = path.relative(workspaceRoot, workspacePath);

    // Skip .xcodeproj files
    if (workspacePath.endsWith(".xcodeproj") || workspacePath.includes(".xcodeproj/")) {
      if (workspacePath.includes(".xcodeproj/project.xcworkspace")) {
        return false; // Don't skip project.xcworkspace files
      }
      return true; // Skip .xcodeproj files
    }

    // Skip standalone .xcworkspace files
    if (workspacePath.includes(".xcworkspace")) {
      return true; // Skip all other .xcworkspace files
    }

    // Skip workspaces that are too deep (likely test fixtures or dependencies)
    if (relativePath.split(path.sep).length > 6) {
      return true;
    }

    // Skip obvious dependency/vendor directories
    const skipPatterns = [
      /\/node_modules\//,
      /\/\.git\//,
      /\/\.build\//,
      /\/DerivedData\//,
      /\/Pods\//,
      /\/\.pod\//,
      /\/vendor\//,
      /\/third_party\//,
      /\/external\//,
      /\/deps\//,
      /\/\.dependencies\//,
      /\/\.bazel\//,
      /\/bazel-out\//,
      /\/bazel-bin\//,
      /\/bazel-testlogs\//,
      /\/\.xcode\//,
      /\/build\//i,
      /\/temp\//i,
      /\/tmp\//i,
    ];

    for (const pattern of skipPatterns) {
      if (pattern.test(workspacePath)) {
        return true;
      }
    }

    // For Bazel files, only keep ones in reasonable locations
    if (workspacePath.endsWith("BUILD") || workspacePath.endsWith("BUILD.bazel")) {
      // Skip if the parent directory is too generic
      const parentDir = path.basename(path.dirname(workspacePath));
      if (parentDir.length < 3 || /^[0-9]+$/.test(parentDir)) {
        return true;
      }
    }

    return false;
  }

  // Prioritize workspaces by relevance - keep the most important ones
  private prioritizeWorkspaces(workspacePaths: string[]): string[] {
    const workspaceRoot = getWorkspacePath();

    return workspacePaths
      .map((workspacePath) => ({
        path: workspacePath,
        score: this.getWorkspaceRelevanceScore(workspacePath, workspaceRoot),
      }))
      .sort((a, b) => b.score - a.score) // Higher scores first
      .map((item) => item.path);
  }

  // Calculate relevance score for workspace prioritization
  private getWorkspaceRelevanceScore(workspacePath: string, workspaceRoot: string): number {
    let score = 0;
    const relativePath = path.relative(workspaceRoot, workspacePath);
    const pathParts = relativePath.split(path.sep);
    const depth = pathParts.length;
    const parentDir = path.basename(path.dirname(workspacePath));

    // Higher score = more relevant

    // Prefer shallower paths
    score += Math.max(0, 10 - depth);

    // Boost main workspace files
    if (depth === 1) {
      score += 20; // Root level files are most important
    }

    // Boost if it's the current workspace
    if (workspacePath === this.defaultWorkspacePath) {
      score += 50;
    }

    // Boost common important patterns
    if (relativePath.match(/^(Sources?|Packages?|Apps?|Projects?)\//i)) {
      score += 15;
    }

    // Boost if parent directory suggests it's a main component
    const importantNames = ["main", "core", "app", "lib", "framework", "service"];
    if (importantNames.some((name) => parentDir.toLowerCase().includes(name))) {
      score += 10;
    }

    // Penalize test/example directories
    if (relativePath.match(/test|spec|example|demo|sample/i)) {
      score -= 5;
    }

    // Prefer .xcworkspace over .xcodeproj
    if (workspacePath.includes(".xcworkspace")) {
      score += 5;
    }

    return score;
  }

  // Add a workspace to the tree and refresh the UI efficiently
  private addWorkspace(workspacePath: string): void {
    // Check workspace limit first
    if (this.workspaces.length >= this.MAX_WORKSPACES) {
      return;
    }

    // First check if this workspace is already in the main list
    if (this.workspaces.some((w) => w.workspacePath === workspacePath)) {
      return;
    }

    // Filter out irrelevant workspaces
    if (this.shouldSkipWorkspace(workspacePath)) {
      return;
    }

    // Create the new workspace item
    const isCurrentWorkspace = workspacePath === this.defaultWorkspacePath;
    const loadingStateKey = `${workspacePath}:regular`;

    // Only show loading indicator for current workspace or items in recents section
    const shouldShowLoading = isCurrentWorkspace && this.loadingItems.get(loadingStateKey);

    // Check if this is the currently selected workspace (for folder icon)
    const isSelectedWorkspace = workspacePath === this.defaultWorkspacePath;

    const workspaceItem = new WorkspaceGroupTreeItem({
      workspacePath: workspacePath,
      provider: this,
      isLoading: shouldShowLoading || false,
    });

    // Add the new item to the regular workspaces list
    this.workspaces.push(workspaceItem);

    // Mark workspaces as needing sort (defer sorting until needed)
    this.workspacesSorted = false;

    // Invalidate cache since workspace list changed
    this.invalidateFilterCache();

    // Use throttled refresh for better performance
    this.throttledRefresh();

    // If this is the current workspace, add it to recents
    if (isCurrentWorkspace) {
      this.addToRecentWorkspaces(workspacePath);
    }
  }

  // Add workspace to specific section and trigger immediate UI update for that section
  private addWorkspaceToSection(
    workspacePath: string,
    expectedSection: string,
    discoveredWorkspaces?: Set<string>,
  ): void {
    // Check workspace limit first to prevent performance issues
    if (this.workspaces.length >= this.MAX_WORKSPACES) {
      console.warn(`🚫 Workspace limit reached (${this.MAX_WORKSPACES}), skipping: ${workspacePath}`);
      return;
    }

    // First check if this workspace is already in the main list
    if (this.workspaces.some((w) => w.workspacePath === workspacePath)) {
      return;
    }

    // Check if this workspace was already discovered in this session
    if (discoveredWorkspaces?.has(workspacePath)) {
      return;
    }

    // Filter out obviously irrelevant workspaces to improve quality
    if (this.shouldSkipWorkspace(workspacePath)) {
      return;
    }

    // Add to discovered set for this session
    if (discoveredWorkspaces) {
      discoveredWorkspaces.add(workspacePath);
    }

    // Verify the workspace actually belongs to the expected section
    const actualSection = this.getWorkspaceCategory(workspacePath);
    if (actualSection !== expectedSection) {
      // If it doesn't match, use the regular addWorkspace method
      this.addWorkspace(workspacePath);
      return;
    }

    // Create the new workspace item
    const isCurrentWorkspace = workspacePath === this.defaultWorkspacePath;
    const loadingStateKey = `${workspacePath}:regular`;

    // Only show loading indicator for current workspace or items in recents section
    const shouldShowLoading = isCurrentWorkspace && this.loadingItems.get(loadingStateKey);

    const workspaceItem = new WorkspaceGroupTreeItem({
      workspacePath: workspacePath,
      provider: this,
      isLoading: shouldShowLoading || false,
    });

    // Add the new item to the regular workspaces list
    this.workspaces.push(workspaceItem);

    // Mark workspaces as needing sort (defer sorting until needed)
    this.workspacesSorted = false;

    // Invalidate cache since workspace list changed
    this.invalidateFilterCache();

    // Use throttled refresh to avoid excessive UI updates during bulk loading
    this.throttledRefresh();

    // If this is the current workspace, add it to recents
    if (isCurrentWorkspace) {
      this.addToRecentWorkspaces(workspacePath);
    }

    // Save to cache periodically as we discover new workspaces
    // Throttled to avoid too many disk writes
    this.throttledCacheSave();
  }

  // Sort workspaces by type (workspace, package, bazel) and then by name - HIGHLY OPTIMIZED
  private sortWorkspaces(): void {
    const len = this.workspaces.length;
    if (len === 0) {
      return; // No need to sort empty array
    }

    if (len === 1) {
      return; // Single item doesn't need sorting
    }

    // Performance timing for large sorts
    const start = performance.now();

    // Pre-compute category and display name for each workspace to avoid repeated calculations
    const workspaceData = new Array(len);
    for (let i = 0; i < len; i++) {
      const workspace = this.workspaces[i];
      const metadata = this.getCachedWorkspaceMetadata(workspace.workspacePath);
      workspaceData[i] = {
        workspace,
        category: metadata.category,
        displayName: metadata.displayName,
      };
    }

    // For very large lists, use bucket sort by category first (more efficient)
    if (len > 500) {
      this.bucketSortWorkspaces(workspaceData);
    } else {
      // Standard sort for smaller lists
      workspaceData.sort((a, b) => {
        // First sort by category (integer comparison - fastest)
        const categoryDiff = a.category - b.category;
        if (categoryDiff !== 0) {
          return categoryDiff;
        }

        // Then sort alphabetically by display name
        return a.displayName.localeCompare(b.displayName);
      });
    }

    // Update the workspaces array with sorted order
    for (let i = 0; i < len; i++) {
      this.workspaces[i] = workspaceData[i].workspace;
    }

    const end = performance.now();
    // Only warn about extremely slow sorts
    if (end - start > 500) {
      console.warn(`Slow workspace sort: ${Math.round(end - start)}ms for ${len} workspaces`);
    }
  }

  // Bucket sort optimization for very large workspace lists
  private bucketSortWorkspaces(
    workspaceData: Array<{ workspace: WorkspaceGroupTreeItem; category: number; displayName: string }>,
  ): void {
    // Create buckets for each category
    const buckets: Map<
      number,
      Array<{ workspace: WorkspaceGroupTreeItem; category: number; displayName: string }>
    > = new Map();

    // Distribute items into buckets
    for (const item of workspaceData) {
      let bucket = buckets.get(item.category);
      if (!bucket) {
        bucket = [];
        buckets.set(item.category, bucket);
      }
      bucket.push(item);
    }

    // Sort each bucket and concatenate
    let index = 0;
    for (const category of [1, 2, 3, 4]) {
      // category order
      const bucket = buckets.get(category);
      if (bucket) {
        // Sort bucket alphabetically
        bucket.sort((a, b) => a.displayName.localeCompare(b.displayName));

        // Copy back to original array
        for (const item of bucket) {
          workspaceData[index++] = item;
        }
      }
    }
  }

  // Optimized caching system for workspace metadata
  private workspaceMetadataCache = new Map<string, { category: number; displayName: string }>();

  private getCachedWorkspaceMetadata(workspacePath: string): { category: number; displayName: string } {
    let metadata = this.workspaceMetadataCache.get(workspacePath);
    if (metadata) {
      return metadata;
    }

    // Calculate both category and display name in one pass for efficiency
    let category: number;
    let displayName: string;

    // Fast path checks using efficient string operations
    if (workspacePath.endsWith("Package.swift")) {
      category = 2; // Packages second
      displayName = path.basename(path.dirname(workspacePath)).toLowerCase();
    } else if (workspacePath.endsWith("BUILD.bazel") || workspacePath.endsWith("BUILD")) {
      category = 3; // Bazel third
      displayName = path.basename(path.dirname(workspacePath)).toLowerCase();
    } else {
      // Check for Xcode workspaces/projects
      const lastDot = workspacePath.lastIndexOf(".");
      if (lastDot !== -1) {
        const extension = workspacePath.slice(lastDot);
        if (
          extension === ".xcworkspace" ||
          extension === ".xcodeproj" ||
          workspacePath.includes(".xcworkspace/") ||
          workspacePath.includes(".xcodeproj/")
        ) {
          category = 1; // Workspaces (including projects) first

          // Extract project name more efficiently
          const lastSlash = workspacePath.lastIndexOf("/", lastDot);
          if (lastSlash !== -1) {
            displayName = workspacePath.slice(lastSlash + 1, lastDot).toLowerCase();
          } else {
            displayName = workspacePath.slice(0, lastDot).toLowerCase();
          }
        } else {
          category = 4; // Other files last
          displayName = path.basename(workspacePath).toLowerCase();
        }
      } else {
        category = 4; // Other files last
        displayName = path.basename(workspacePath).toLowerCase();
      }
    }

    metadata = { category, displayName };

    // Cache for future use - limit cache size to prevent memory issues
    if (this.workspaceMetadataCache.size > 2000) {
      // Clear oldest entries (simple LRU)
      const keysToDelete = Array.from(this.workspaceMetadataCache.keys()).slice(0, 500);
      for (const key of keysToDelete) {
        this.workspaceMetadataCache.delete(key);
      }
    }

    this.workspaceMetadataCache.set(workspacePath, metadata);
    return metadata;
  }

  // Helper to get the category of a workspace
  private getWorkspaceCategory(workspacePath: string): string {
    if (
      workspacePath.endsWith(".xcworkspace") ||
      workspacePath.includes(".xcworkspace/") ||
      workspacePath.endsWith(".xcodeproj") ||
      workspacePath.includes(".xcodeproj/")
    ) {
      return "workspace";
    } else if (workspacePath.endsWith("Package.swift")) {
      return "package";
    } else if (workspacePath.endsWith("BUILD.bazel") || workspacePath.endsWith("BUILD")) {
      return "bazel";
    }
    return "other";
  }

  // Group workspaces by type into collapsible sections
  private getSectionedWorkspaces(): WorkspaceSectionTreeItem[] {
    const start = performance.now();

    // Make sure workspaces are sorted (only if needed) - skip for small lists or when already sorted
    if (!this.workspacesSorted && this.workspaces.length > 1) {
      this.sortWorkspaces();
      this.workspacesSorted = true;
    }

    // Group by category
    const workspacesByCategory = new Map<string, WorkspaceGroupTreeItem[]>();

    // Use cached filtered data if available, otherwise compute
    let filteredRecentWorkspaces: WorkspaceGroupTreeItem[];
    let filteredWorkspaces: WorkspaceGroupTreeItem[];

    if (this.isSearchActive) {
      // Ensure filtered cache is computed efficiently
      if (
        this.searchTerm !== this.lastComputedSearchTerm ||
        this.cachedFilteredRecentWorkspaces === null ||
        this.cachedFilteredWorkspaces === null
      ) {
        this.computeFilteredCache();
      }
      filteredRecentWorkspaces = this.cachedFilteredRecentWorkspaces || [];
      filteredWorkspaces = this.cachedFilteredWorkspaces || [];
    } else {
      // No search active, use original data (fastest path)
      filteredRecentWorkspaces = this.recentWorkspaces;
      filteredWorkspaces = this.workspaces;
    }

    // Add recents section if we have any
    if (filteredRecentWorkspaces.length > 0) {
      workspacesByCategory.set("recent", [...filteredRecentWorkspaces]);
    }

    // Process regular workspaces efficiently - batch by category for better performance
    const len = filteredWorkspaces.length;
    for (let i = 0; i < len; i++) {
      const workspace = filteredWorkspaces[i];
      const category = this.getWorkspaceCategory(workspace.workspacePath);

      let categoryWorkspaces = workspacesByCategory.get(category);
      if (!categoryWorkspaces) {
        categoryWorkspaces = [];
        workspacesByCategory.set(category, categoryWorkspaces);
      }

      categoryWorkspaces.push(workspace);
    }

    // Create section items for each category
    const sections: WorkspaceSectionTreeItem[] = [];

    // Define the order we want categories to appear
    const categoryOrder = ["recent", "workspace", "package", "bazel", "other"];

    // Pre-compute category counts only if needed (during search)
    const categoryTotalCounts = new Map<string, number>();
    if (this.isSearchActive) {
      // Count categories efficiently in one pass
      for (const workspace of this.workspaces) {
        const category = this.getWorkspaceCategory(workspace.workspacePath);
        categoryTotalCounts.set(category, (categoryTotalCounts.get(category) || 0) + 1);
      }
    }

    for (const category of categoryOrder) {
      const workspaces = workspacesByCategory.get(category) || [];

      // Show sections even if empty during loading (except "recent")
      const shouldShowEmptySection = category !== "recent" && this.sectionsLoading.has(category);

      if (workspaces.length > 0 || shouldShowEmptySection) {
        // Get total count efficiently
        let totalCount: number | undefined;
        if (this.isSearchActive) {
          totalCount = category === "recent" ? this.recentWorkspaces.length : categoryTotalCounts.get(category);
        }

        const isLoading = this.sectionsLoading.has(category);
        sections.push(new WorkspaceSectionTreeItem(category, workspaces, this.searchTerm, totalCount, isLoading));
      }
    }

    const end = performance.now();
    const duration = Math.round(end - start);

    // Only log very slow operations
    if (duration > 200) {
      console.warn(`Slow getSectionedWorkspaces: ${duration}ms for ${this.workspaces.length} workspaces`);
    }

    return sections;
  }

  // Initialize workspaces with immediate cache load, then background refresh
  private async initializeWorkspaces(): Promise<void> {
    try {
      // 1. FIRST: Try to load from cache immediately for instant display
      const cacheLoaded = await this.loadFromPersistentCache();

      if (cacheLoaded) {
        // Update UI immediately with cached data
        this._onDidChangeTreeData.fire(undefined);
      }

      // Start background search immediately to discover new workspaces
      // This ensures cache stays fresh and discovers new workspaces
      void this.loadWorkspacesStreamingly();
    } catch (error) {
      console.error("Failed to initialize workspaces:", error);
      // Fallback to normal loading
      void this.loadWorkspacesStreamingly();
    }
  }

  // Load workspaces from persistent cache
  private async loadFromPersistentCache(): Promise<boolean> {
    try {
      const globalState = (this.context as any)?._context?.globalState;
      const cachedData = globalState?.get(this.persistentCacheKey) as WorkspaceCacheData | undefined;

      if (!cachedData || cachedData.version !== this.persistentCacheVersion) {
        return false; // Cache is invalid or wrong version
      }

      // Check if cache is for the same workspace root
      const currentWorkspaceRoot = getWorkspacePath();
      if (cachedData.workspaceRoot !== currentWorkspaceRoot) {
        return false; // Cache is for different workspace
      }

      // Check if cache is not too old (7 days)
      const cacheMaxAge = 7 * 24 * 60 * 60 * 1000; // 7 days in ms
      if (Date.now() - cachedData.timestamp > cacheMaxAge) {
        return false; // Cache is too old
      }

      // If cache is too large, prioritize and truncate
      let workspacePathsToLoad = cachedData.workspacePaths.filter(
        (path: string) => path && !this.shouldSkipWorkspace(path),
      );

      if (workspacePathsToLoad.length > this.MAX_WORKSPACES) {
        workspacePathsToLoad = this.prioritizeWorkspaces(workspacePathsToLoad).slice(0, this.MAX_WORKSPACES);

        // Auto-save the optimized cache to prevent this issue next time
        setTimeout(() => void this.saveToPersistentCache(), 5000);
      }

      // Create workspace items from cached paths
      this.workspaces = workspacePathsToLoad.map(
        (workspacePath: string) =>
          new WorkspaceGroupTreeItem({
            workspacePath,
            provider: this,
            isRecent: false,
          }),
      );

      // Restore recent workspaces with limit
      this.recentWorkspacesStorage = cachedData.recentWorkspacePaths
        .filter((path: string) => path)
        .slice(0, this.MAX_RECENT_WORKSPACES); // Limit recent workspaces too

      this.recentWorkspaces = this.recentWorkspacesStorage.map(
        (path: string) =>
          new WorkspaceGroupTreeItem({
            workspacePath: path,
            provider: this,
            isRecent: true,
          }),
      );

      // Sort workspaces
      this.sortWorkspaces();
      this.workspacesSorted = true;

      // Update UI with cached data
      this.invalidateDataCache();
      this._onDidChangeTreeData.fire(undefined);

      return true;
    } catch (error) {
      console.error("Failed to load persistent cache:", error);
      return false;
    }
  }

  // Save workspaces to persistent cache
  private async saveToPersistentCache(): Promise<void> {
    try {
      if (!this.context) {
        return;
      }

      // Prioritize and limit workspaces before saving to cache
      let workspacePathsToSave = this.workspaces.map((w) => w.workspacePath);

      if (workspacePathsToSave.length > this.MAX_WORKSPACES) {
        console.log(
          `🔧 Optimizing cache: reducing ${workspacePathsToSave.length} workspaces to best ${this.MAX_WORKSPACES}`,
        );
        workspacePathsToSave = this.prioritizeWorkspaces(workspacePathsToSave).slice(0, this.MAX_WORKSPACES);
      }

      const cacheData: WorkspaceCacheData = {
        version: this.persistentCacheVersion,
        timestamp: Date.now(),
        workspacePaths: workspacePathsToSave,
        recentWorkspacePaths: this.recentWorkspacesStorage.slice(0, this.MAX_RECENT_WORKSPACES),
        workspaceRoot: getWorkspacePath(),
      };

      await (this.context as any)._context.globalState.update(this.persistentCacheKey, cacheData);
    } catch (error) {
      console.error("Failed to save persistent cache:", error);
    }
  }

  // Clear persistent cache (useful for debugging or cache corruption)
  public async clearPersistentCache(): Promise<void> {
    try {
      if (this.context) {
        await (this.context as any)._context.globalState.update(this.persistentCacheKey, undefined);
      }
    } catch (error) {
      console.error("Failed to clear persistent cache:", error);
    }
  }

  // Parse individual BUILD.bazel file on-demand with caching
  public async getCachedBazelPackage(buildFilePath: string): Promise<BazelPackage | null> {
    const now = Date.now();
    const cacheMaxAge = 30000; // 30 seconds cache per file

    // Validate file path
    if (!buildFilePath || typeof buildFilePath !== "string" || buildFilePath.length === 0) {
      return null;
    }

    // Check if we have a cached version
    const cachedTimestamp = this.bazelFileCacheTimestamps.get(buildFilePath);
    const cachedPackage = this.cachedBazelFiles.get(buildFilePath);

    if (cachedPackage !== undefined && cachedTimestamp && now - cachedTimestamp < cacheMaxAge) {
      return cachedPackage; // Return cached result
    }

    try {
      const bazelPackage = await parseBazelBuildFile(buildFilePath);

      // Cache the result (even if null)
      this.cachedBazelFiles.set(buildFilePath, bazelPackage);
      this.bazelFileCacheTimestamps.set(buildFilePath, now);

      return bazelPackage;
    } catch (error) {
      console.error(`Failed to parse BUILD.bazel file: ${buildFilePath}`, error);

      // Cache the failure to avoid repeated attempts
      this.cachedBazelFiles.set(buildFilePath, null);
      this.bazelFileCacheTimestamps.set(buildFilePath, now);

      return null;
    }
  }

  // Load workspaces with streaming updates to the UI
  public async loadWorkspacesStreamingly(): Promise<void> {
    if (this.isLoadingWorkspaces) {
      return; // Prevent multiple concurrent loading sessions
    }

    this.isLoadingWorkspaces = true;

    try {
      // 1. FIRST: Try to load from persistent cache for instant display
      const cacheLoaded = await this.loadFromPersistentCache();

      if (!cacheLoaded) {
        // No cache available, reset workspaces list
        this.workspaces = [];

        // Add current workspace if available
        if (this.context) {
          const cachedWorkspacePath = getCurrentXcodeWorkspacePath(this.context);
          if (cachedWorkspacePath) {
            this.addWorkspace(cachedWorkspacePath);
          }
        }
      }

      // 2. THEN: Start real search in background (always, even if cache loaded)
      console.log(`🔄 Starting background workspace discovery...`);

      // Reset sections loading state for real search
      this.sectionsLoading = new Set<string>(["workspace", "package", "bazel"]);
      this.subsectionLoadingCount = new Map<string, number>([
        ["workspace", 2], // project.xcworkspace search (counts as 2 for compatibility)
        ["package", 1], // Package.swift search
        ["bazel", 1], // BUILD.bazel search
      ]);

      // Start real search without awaiting - this runs in background
      void this.streamingWorkspaceSearchAndCache();

      // Update context variable for welcome screen
      void vscode.commands.executeCommand(
        "setContext",
        "sweetpad.workspaces.noWorkspaces",
        this.workspaces.length === 0,
      );
    } catch (error) {
      commonLogger.error("Failed to load workspaces", { error });
      void vscode.commands.executeCommand("setContext", "sweetpad.workspaces.noWorkspaces", true);
      this.isLoadingWorkspaces = false;
    }
  }

  // Perform workspace search and save to cache when complete
  private async streamingWorkspaceSearchAndCache(): Promise<void> {
    // First, collect all discovered workspaces in a temporary set to avoid duplicates
    const discoveredWorkspaces = new Set<string>(this.workspaces.map((w) => w.workspacePath));

    // Perform the real search
    await this.streamingWorkspaceSearch(discoveredWorkspaces);

    // Save updated cache after search completes
    await this.saveToPersistentCache();
  }

  // Perform workspace search with parallel loading and immediate UI updates
  private async streamingWorkspaceSearch(discoveredWorkspaces?: Set<string>): Promise<void> {
    const workspace = getWorkspacePath();

    try {
      // Immediately show empty sections with loading indicators
      this._onDidChangeTreeData.fire(undefined);

      // Debug logging can be enabled if needed
      // console.log(`🚀 Starting parallel searches for sections:`, Array.from(this.sectionsLoading));
      // console.log(`📊 Subsection counts:`, Object.fromEntries(this.subsectionLoadingCount));

      // Start all searches in parallel and track their completion
      const searchPromises = [
        // Search for Package.swift files (SPM packages)
        this.findFilesIncrementallyWithCallback({
          directory: workspace,
          depth: 4,
          maxResults: 30, // Limit to prevent performance issues
          matcher: (file) => file.name === "Package.swift",
          processFile: (filePath) => this.addWorkspaceToSection(filePath, "package", discoveredWorkspaces),
          onComplete: () => {
            // console.log(`📦 Package search completed`);
            this.onSectionLoadComplete("package");
          },
        }),

        // Search for project.xcworkspace files inside .xcodeproj directories only
        this.findFilesIncrementallyWithCallback({
          directory: workspace,
          depth: 4,
          maxResults: 20, // Limit workspace files
          matcher: (file) => file.name === "project.xcworkspace",
          processFile: (filePath) => {
            // Only process if it's inside a .xcodeproj directory
            if (filePath.includes(".xcodeproj/project.xcworkspace")) {
              this.addWorkspaceToSection(filePath, "workspace", discoveredWorkspaces);
            }
          },
          onComplete: () => {
            // console.log(`🏢 Workspace (project.xcworkspace) search completed`);
            // Since we removed the .xcodeproj search, we need to decrement twice
            this.onSectionLoadComplete("workspace");
            this.onSectionLoadComplete("workspace");
          },
        }),

        // Search for Bazel BUILD files
        this.findFilesIncrementallyWithCallback({
          directory: workspace,
          depth: 4,
          maxResults: 50, // Limit Bazel files
          matcher: (file) => file.name === "BUILD.bazel" || file.name === "BUILD",
          processFile: (filePath) => this.addWorkspaceToSection(filePath, "bazel", discoveredWorkspaces),
          onComplete: () => {
            // console.log(`⚙️ Bazel search completed`);
            this.onSectionLoadComplete("bazel");
          },
        }),
      ];

      // Wait for all searches to complete
      await Promise.all(searchPromises);

      // Mark overall loading as complete
      this.isLoadingWorkspaces = false;
      this._onDidChangeTreeData.fire(undefined);

      // Safety mechanism: Force complete any remaining sections after a timeout
      setTimeout(() => {
        if (this.sectionsLoading.size > 0) {
          // console.warn(`⚠️ Forcing completion of stuck sections:`, Array.from(this.sectionsLoading));
          this.sectionsLoading.clear();
          this.subsectionLoadingCount.clear();
          this._onDidChangeTreeData.fire(undefined);
        }
      }, 15000); // 15 second safety timeout
    } catch (error) {
      commonLogger.error("Error in workspace search", { error });
      this.isLoadingWorkspaces = false;
      this.sectionsLoading.clear();
      this._onDidChangeTreeData.fire(undefined);
    }
  }

  // Called when a specific section type has finished loading
  private onSectionLoadComplete(sectionType: string): void {
    // Decrement the count of remaining subsections for this section
    const currentCount = this.subsectionLoadingCount.get(sectionType) || 0;
    const newCount = Math.max(0, currentCount - 1);
    this.subsectionLoadingCount.set(sectionType, newCount);

    // console.log(`🔍 Section ${sectionType} completed search: ${currentCount} -> ${newCount} remaining`);

    // Only mark section as complete when all its subsections are done
    if (newCount === 0) {
      this.sectionsLoading.delete(sectionType);
      // console.log(`✅ Section ${sectionType} fully loaded, removing loading indicator`);
      // Trigger UI update to remove loading indicator for this section
      this._onDidChangeTreeData.fire(undefined);
    }
  }

  // Helper method to incrementally find and process files with completion callback
  private async findFilesIncrementallyWithCallback(options: {
    directory: string;
    matcher: (file: Dirent) => boolean;
    processFile: (filePath: string) => void;
    onComplete?: () => void;
    ignore?: string[];
    depth?: number;
    maxResults?: number;
  }): Promise<void> {
    try {
      await this.findFilesIncrementally({
        directory: options.directory,
        matcher: options.matcher,
        processFile: options.processFile,
        ignore: options.ignore,
        depth: options.depth,
        maxResults: options.maxResults,
      });
    } catch (error) {
      // console.error(`🚫 Search failed for directory ${options.directory}:`, error);
      commonLogger.error("Search failed", { directory: options.directory, error });
    } finally {
      // Always call completion callback, even if search failed
      if (options.onComplete) {
        // console.log(`🔄 Calling completion callback for ${options.directory}`);
        options.onComplete();
      }
    }
  }

  // Helper method to incrementally find and process files
  private async findFilesIncrementally(options: {
    directory: string;
    matcher: (file: Dirent) => boolean;
    processFile: (filePath: string) => void;
    ignore?: string[];
    depth?: number;
    maxResults?: number;
  }): Promise<void> {
    const ignore = options.ignore ?? [];
    const depth = options.depth ?? 0;
    let processedCount = 0;

    try {
      const files = await fs.readdir(options.directory, { withFileTypes: true });

      // Process matching files immediately with limit check
      for (const file of files) {
        // Stop if we've hit the max results limit
        if (options.maxResults && processedCount >= options.maxResults) {
          break;
        }

        const fullPath = path.join(options.directory, file.name);

        if (options.matcher(file)) {
          options.processFile(fullPath);
          processedCount++;
        }

        // Queue up directory searches to run in parallel
        if (file.isDirectory() && !ignore.includes(file.name) && depth > 0) {
          const remainingResults = options.maxResults ? Math.max(0, options.maxResults - processedCount) : undefined;

          // Only continue searching subdirectories if we haven't hit the limit
          if (!options.maxResults || processedCount < options.maxResults) {
            void this.findFilesIncrementally({
              directory: fullPath,
              matcher: options.matcher,
              processFile: options.processFile,
              ignore: options.ignore,
              depth: depth - 1,
              maxResults: remainingResults,
            });
          }
        }
      }
    } catch (error) {
      commonLogger.error(`Error searching directory: ${options.directory}`, { error });
    }
  }

  async getChildren(element?: vscode.TreeItem): Promise<vscode.TreeItem[]> {
    // Start loading if needed - but first try cache
    if (this.workspaces.length === 0 && !this.isLoadingWorkspaces && !element) {
      this.initializeWorkspaces();
    }

    // Root level - show sections
    if (!element) {
      const sections = this.getSectionedWorkspaces();
      const results: vscode.TreeItem[] = [];

      // Add search status indicator at the top when filtering is active
      if (this.isSearchActive && this.searchTerm.length > 0) {
        const searchStatusItem = new vscode.TreeItem(
          `🔍 Filtering: "${this.searchTerm}"`,
          vscode.TreeItemCollapsibleState.None,
        );
        searchStatusItem.iconPath = new vscode.ThemeIcon("search-stop");
        searchStatusItem.contextValue = "search-status";
        searchStatusItem.tooltip = `Click to clear search filter`;
        searchStatusItem.command = {
          command: "sweetpad.build.clearSearch",
          title: "Clear Search",
          arguments: [],
        };
        results.push(searchStatusItem);
      }

      // Add sections
      results.push(...sections);

      // Add a loading indicator if we're still searching
      if (this.isLoadingWorkspaces) {
        const loadingItem = new vscode.TreeItem("Searching for more builds...", vscode.TreeItemCollapsibleState.None);
        loadingItem.iconPath = new vscode.ThemeIcon("loading~spin");
        results.push(loadingItem);
      }

      void vscode.commands.executeCommand(
        "setContext",
        "sweetpad.workspaces.noWorkspaces",
        this.workspaces.length === 0,
      );

      return results;
    }

    // For section items, return their workspaces
    if (element instanceof WorkspaceSectionTreeItem) {
      return element.workspaces;
    }

    // For workspace items, return schemes/targets
    if (element instanceof WorkspaceGroupTreeItem) {
      // Check if this is a Bazel workspace first
      const isBazelWorkspace = element.workspacePath.endsWith("BUILD.bazel") || element.workspacePath.endsWith("BUILD");

      // For Bazel workspaces, allow expansion even if not in Recents
      if (isBazelWorkspace) {
        // Parse the specific BUILD.bazel file on-demand (only when clicked)
        const buildFile = element.workspacePath;
        const bazelPackage = await this.getCachedBazelPackage(buildFile);

        if (bazelPackage) {
          // Create tree items for each target
          const targetItems = bazelPackage.targets.map((target) => {
            const bazelTreeItem = new BazelTreeItem({
              target,
              package: bazelPackage,
              provider: this,
              workspacePath: element.workspacePath,
            });

            // Cache the target for command lookup
            this.currentBazelTargets.set(target.buildLabel, bazelTreeItem);

            return bazelTreeItem;
          });

          // Apply search filter if active - optimized
          if (this.isSearchActive && this.searchTerm.length > 0) {
            const searchTerm = this.searchTerm;
            const filteredTargets: BazelTreeItem[] = [];

            for (const item of targetItems) {
              if (item.target.name.toLowerCase().indexOf(searchTerm) !== -1) {
                filteredTargets.push(item);
              }
            }

            return filteredTargets;
          }

          return targetItems;
        } else {
          return [];
        }
      }

      // For non-Bazel workspaces, only load schemes if in Recents section
      else if (element.isRecent) {
        // Handle regular Xcode/SPM workspaces with schemes
        const cacheKey = `${element.workspacePath}:${this.searchTerm}`;
        let schemes = this.cachedSchemesForWorkspaces.get(cacheKey);

        if (!schemes) {
          // Load schemes directly for this specific workspace without using the cache
          const allSchemes = await this.getSchemesDirectly(element.workspacePath);
          schemes = await this.filterSchemes(allSchemes);

          // Cache the filtered results
          this.cachedSchemesForWorkspaces.set(cacheKey, schemes);
        }

        return schemes;
      } else {
        // Return empty array for non-recent items to prevent them from expanding
        return [];
      }
    }

    return [];
  }

  async getTreeItem(element: vscode.TreeItem): Promise<vscode.TreeItem> {
    return element;
  }

  async getWorkspacePaths(): Promise<WorkspaceGroupTreeItem[]> {
    if (this.workspaces.length === 0 && !this.isLoadingWorkspaces) {
      await this.loadWorkspacesStreamingly();
    }
    return this.workspaces;
  }

  async getSchemes(workspacePath?: string): Promise<BuildTreeItem[]> {
    // If a specific workspace path is provided, load schemes directly for that workspace
    if (workspacePath) {
      const schemes = await this.getSchemesDirectly(workspacePath);
      return await this.filterSchemes(schemes);
    }

    // Otherwise, use the regular method with the current workspace path
    let schemes: XcodeScheme[] = [];
    try {
      // This will get schemes for the current workspace path
      schemes = await this.buildManager.getSchemas();
    } catch (error) {
      commonLogger.error("Failed to get schemes", {
        error: error,
      });
    }

    if (schemes.length === 0) {
      // Display welcome screen with explanation what to do.
      // See "viewsWelcome": [ {"view": "sweetpad.build.view", ...} ] in package.json
      vscode.commands.executeCommand("setContext", "sweetpad.build.noSchemes", true);
    }

    // return list of schemes with the specific workspace path
    const buildTreeItems = schemes.map(
      (scheme) =>
        new BuildTreeItem({
          scheme: scheme.name,
          collapsibleState: vscode.TreeItemCollapsibleState.None,
          provider: this,
          workspacePath: workspacePath || this.defaultWorkspacePath,
        }),
    );

    return await this.filterSchemes(buildTreeItems);
  }

  // Get schemes directly from xcodebuild without using the cache
  async getSchemesDirectly(workspacePath: string): Promise<BuildTreeItem[]> {
    if (!workspacePath) {
      return [];
    }

    let schemes: XcodeScheme[] = [];
    try {
      // Call getSchemes directly with this workspace path
      schemes = await getSchemes({
        xcworkspace: workspacePath,
      });
    } catch (error) {
      commonLogger.error("Failed to get schemes for workspace", {
        workspacePath,
        error,
      });
      return [];
    }

    // Create scheme items with explicit workspace path
    return schemes.map(
      (scheme) =>
        new BuildTreeItem({
          scheme: scheme.name,
          collapsibleState: vscode.TreeItemCollapsibleState.None,
          provider: this,
          workspacePath: workspacePath,
        }),
    );
  }

  // Track loading state for workspace items - updated to use the tree item directly
  setItemLoading(item: WorkspaceGroupTreeItem, isLoading: boolean): void {
    // Clear previous loading item if exists
    if (this.loadingItem && this.loadingItem !== item) {
      this.loadingItem.isLoading = false;
      this.refreshTreeItem(this.loadingItem);
    }

    // Only allow loading state for recent items
    if (!item.isRecent) {
      return;
    }

    // Update the loading state just for this specific item
    item.isLoading = isLoading;

    if (isLoading) {
      // Set this as the current loading item
      this.loadingItem = item;
      this.loadingItems.set(item.uniqueId, true);
    } else {
      // Clear current loading item
      this.loadingItem = null;
      this.loadingItems.delete(item.uniqueId);
    }

    // Only refresh the specific item that was changed
    this.refreshTreeItem(item);
  }

  // Clear all loading states
  clearAllLoadingStates(): void {
    if (this.loadingItem) {
      this.loadingItem.isLoading = false;
      this.refreshTreeItem(this.loadingItem);
      this.loadingItem = null;
    }
    this.loadingItems.clear();
  }
}
