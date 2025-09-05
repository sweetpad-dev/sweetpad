import path from "node:path";
import * as vscode from "vscode";
import type { XcodeScheme } from "../common/cli/scripts";
import { getSchemes } from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { commonLogger } from "../common/logger";
import type { BuildManager } from "./manager";
import { getCurrentXcodeWorkspacePath, getWorkspacePath } from "./utils";
import { detectXcodeWorkspacesPaths, getBazelPackages, type BazelTarget, type BazelPackage } from "./utils";
import { ExtensionError } from "../common/errors";
import * as fs from "node:fs/promises";
import type { Dirent } from "node:fs";

type WorkspaceEventData = WorkspaceGroupTreeItem | undefined | null;
type BuildEventData = BuildTreeItem | undefined | null;

export class WorkspaceGroupTreeItem extends vscode.TreeItem {
  public provider: WorkspaceTreeProvider;
  public workspacePath: string;
  public isRecent: boolean;
  public isLoading: boolean = false;
  public uniqueId: string;

  constructor(options: { workspacePath: string; provider: WorkspaceTreeProvider; isRecent?: boolean; isLoading?: boolean }) {
    // Extract just the xcodeproj name from the full path
    const match = options.workspacePath.match(/([^/]+)\.xcodeproj/);
    
    let displayName = "";
    
    // For Package.swift files, include the parent folder name
    if (path.basename(options.workspacePath) === "Package.swift") {
      // Get the parent folder name and possibly grandparent folder for more context
      const parentDir = path.dirname(options.workspacePath);
      const parentFolderName = path.basename(parentDir);
      
      // If the parent folder name is too generic (like "Sources"), include one more level up
      const grandParentDir = path.dirname(parentDir);
      const grandParentFolderName = path.basename(grandParentDir);
      
      // Determine how many parent folders to include based on the workspace path depth
      const workspaceRoot = getWorkspacePath();
      const relativePath = path.relative(workspaceRoot, options.workspacePath);
      const folderDepth = relativePath.split(path.sep).length - 1; // -1 for the filename itself
      
      if (folderDepth > 2) {
        // For deep paths, show more context (without "/Package.swift")
        displayName = `${grandParentFolderName}/${parentFolderName}`;
      } else {
        // For shallow paths, just show immediate parent (without "/Package.swift")
        displayName = parentFolderName;
      }
    } else if (path.basename(options.workspacePath) === "BUILD.bazel" || path.basename(options.workspacePath) === "BUILD") {
      // For Bazel BUILD files, use the parent directory name
      const parentDir = path.dirname(options.workspacePath);
      const parentFolderName = path.basename(parentDir);
      
      // Get context for deeper paths if needed
      const grandParentDir = path.dirname(parentDir);
      const grandParentFolderName = path.basename(grandParentDir);
      
      // Determine how many parent folders to include based on the workspace path depth
      const workspaceRoot = getWorkspacePath();
      const relativePath = path.relative(workspaceRoot, options.workspacePath);
      const folderDepth = relativePath.split(path.sep).length - 1; // -1 for the filename itself
      
      if (folderDepth > 2) {
        // For deep paths, show more context
        displayName = `${grandParentFolderName}/${parentFolderName}`;
      } else {
        // For shallow paths, just show immediate parent
        displayName = parentFolderName;
      }
    } else {
      // For Xcode projects, use existing logic
      displayName = match ? match[1] : path.basename(options.workspacePath);
    }

    // Set collapsible state based on whether it's in the Recents section
    // Only Recents should be expandable to show schemes
    const isRecent = !!options.isRecent;
    const isCurrentWorkspace = options.provider.defaultWorkspacePath === options.workspacePath;
    
    // Items in Recents are expandable, others are not
    const collapsibleState = isRecent 
      ? vscode.TreeItemCollapsibleState.Expanded
      : vscode.TreeItemCollapsibleState.None;

    // What constructs the display of the tree item
    super(displayName, collapsibleState);

    this.workspacePath = options.workspacePath;
    this.provider = options.provider;
    this.isRecent = isRecent;
    this.isLoading = !!options.isLoading;
    
    // Create a unique ID that combines path and whether it's a recent item
    this.uniqueId = `${this.workspacePath}:${this.isRecent ? 'recent' : 'regular'}`;

    let description = "";
    let color: vscode.ThemeColor = new vscode.ThemeColor("foreground");

    // Only show checkmark on selected workspace
    if (isCurrentWorkspace) {
      description = `${description} ✓`;
      color = new vscode.ThemeColor("sweetpad.workspace");
    }
    
    // Add loading indicator
    if (this.isLoading) {
      description = `${description} (loading...)`;
    }

    if (description) {
      this.description = description;
    }
    
    if (path.basename(this.workspacePath) === "Package.swift") {
      this.iconPath = vscode.Uri.joinPath(vscode.Uri.file(this.provider.context?.extensionPath || ""), "images", "spm.png");
    }
    // Use bazel icon for BUILD.bazel files
    else if (path.basename(this.workspacePath) === "BUILD.bazel" || path.basename(this.workspacePath) === "BUILD") {
      this.iconPath = vscode.Uri.joinPath(vscode.Uri.file(this.provider.context?.extensionPath || ""), "images", "bazel.png");
    }
    // Use xcodeproj icon for .xcodeproj files
    else if (this.workspacePath.endsWith(".xcodeproj") || this.workspacePath.includes(".xcodeproj/")) {
      this.iconPath = vscode.Uri.joinPath(vscode.Uri.file(this.provider.context?.extensionPath || ""), "images", "xcodeproj.png");
    }
    // Use xcworkspace icon for .xcworkspace files
    else if (this.workspacePath.endsWith(".xcworkspace") || this.workspacePath.includes(".xcworkspace/") || this.workspacePath.includes("project.xcworkspace")) {
      this.iconPath = vscode.Uri.joinPath(vscode.Uri.file(this.provider.context?.extensionPath || ""), "images", "xcworkspace.png");
    }
    // Default folder icon for other items
    else {
      this.iconPath = new vscode.ThemeIcon("folder", color);
    }

    this.contextValue = "workspace-group";
    this.tooltip = this.workspacePath;
  }
  
  // Set loading state and refresh the UI
  setLoading(loading: boolean): void {
    this.isLoading = loading;
    // Update the loading state in the provider
    if (this.provider) {
      this.provider.setItemLoading(this, loading);
    }
  }
}

// Section header for grouping workspaces
export class WorkspaceSectionTreeItem extends vscode.TreeItem {
  public readonly workspaces: WorkspaceGroupTreeItem[];
  public readonly sectionType: string;
  public readonly isLoading: boolean;
  
  constructor(sectionType: string, workspaces: WorkspaceGroupTreeItem[], searchTerm?: string, totalCount?: number, isLoading?: boolean) {
    // Format the section title with first letter capitalized and make it plural
    let label = sectionType.charAt(0).toUpperCase() + sectionType.slice(1) + (sectionType === "recent" ? "s" : "s");
    
    // Add loading indicator if loading and no workspaces yet
    if (isLoading && workspaces.length === 0) {
      label += " (loading...)";
    }
    // Add search indicator if filtering is active
    else if (searchTerm && searchTerm.length > 0) {
      if (totalCount !== undefined && totalCount > 0) {
        label += ` (${workspaces.length}/${totalCount} filtered)`;
      } else {
        label += ` (${workspaces.length} filtered)`;
      }
    }
    
    // Make sections collapsible and expanded by default
    super(label, vscode.TreeItemCollapsibleState.Expanded);
    
    this.workspaces = workspaces;
    this.sectionType = sectionType;
    this.isLoading = isLoading || false;
    this.contextValue = "workspace-section";
    
    // Use an appropriate icon based on section type, state, and search
    const isFiltered = searchTerm && searchTerm.length > 0;
    const isLoadingEmpty = isLoading && workspaces.length === 0;
    
    if (isLoadingEmpty) {
      // Use loading icon for empty sections that are loading
      this.iconPath = new vscode.ThemeIcon("loading~spin");
    } else if (isFiltered) {
      // Use search icon to indicate filtered results
      this.iconPath = new vscode.ThemeIcon("filter");
    } else if (sectionType === "recent") {
      this.iconPath = new vscode.ThemeIcon("history");
    } else if (sectionType === "workspace") {
      this.iconPath = new vscode.ThemeIcon("multiple-windows");
    } else if (sectionType === "package") {
      this.iconPath = new vscode.ThemeIcon("extensions");
    } else if (sectionType === "bazel") {
      // Use bazel.png icon from images folder, fallback to gear icon if no workspaces
      if (workspaces.length > 0 && workspaces[0]?.provider?.context?.extensionPath) {
        this.iconPath = vscode.Uri.joinPath(vscode.Uri.file(workspaces[0].provider.context.extensionPath), "images", "bazel.png");
      } else {
        this.iconPath = new vscode.ThemeIcon("gear");
      }
    } else {
      this.iconPath = new vscode.ThemeIcon("folder");
    }
  }
}

export class WorkspaceTreeProvider implements vscode.TreeDataProvider<vscode.TreeItem> {
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
  private readonly MAX_RECENT_ITEMS = 5;
  private recentWorkspacesStorage: string[] = [];
  private loadingItems = new Map<string, boolean>();
  private loadingItem: WorkspaceGroupTreeItem | null = null;
  private searchTerm: string = "";
  private isSearchActive: boolean = false;
  
  // Cached filtered data to avoid recomputation
  private cachedFilteredWorkspaces: WorkspaceGroupTreeItem[] | null = null;
  private cachedFilteredRecentWorkspaces: WorkspaceGroupTreeItem[] | null = null;
  private cachedSchemesForWorkspaces = new Map<string, BuildTreeItem[]>();
  
  // Track loading state for each section type  
  private sectionsLoading = new Set<string>(["workspace", "package", "bazel"]);
  
  // Track individual searches within sections (for workspace section which has multiple searches)
  private subsectionLoadingCount = new Map<string, number>([
    ["workspace", 2], // xcworkspace + xcodeproj searches
    ["package", 1],   // Package.swift search
    ["bazel", 1]      // BUILD.bazel search
  ]);
  
  // Throttled UI refresh for better performance during bulk loading
  private refreshThrottleTimer: NodeJS.Timeout | null = null;

  constructor(options: { context: ExtensionContext; buildManager: BuildManager}) {
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

    // Initial workspace loading
    this.loadWorkspacesStreamingly();
  }

  // Public method to refresh a specific tree item
  refreshTreeItem(item: WorkspaceGroupTreeItem | null): void {
    // Only refresh the specific item, not the entire tree
    this._onDidChangeTreeData.fire(item);
  }

  // Cache invalidation methods
  private invalidateFilterCache(): void {
    this.cachedFilteredWorkspaces = null;
    this.cachedFilteredRecentWorkspaces = null;
    this.cachedSchemesForWorkspaces.clear();
  }

  private invalidateDataCache(): void {
    // Called when underlying data changes
    this.invalidateFilterCache();
  }

  // Search functionality with caching
  public setSearchTerm(searchTerm: string): void {
    const previousSearchTerm = this.searchTerm;
    this.searchTerm = searchTerm.toLowerCase();
    this.isSearchActive = searchTerm.length > 0;
    
    // Update context for conditional UI elements
    this.updateSearchContext();
    
    // Only recompute filtered cache if search term actually changed
    if (previousSearchTerm !== this.searchTerm) {
      this.computeFilteredCache();
      // Only fire tree change event, don't call full refresh
      this._onDidChangeTreeData.fire(null);
    }
  }

  public clearSearch(): void {
    if (this.searchTerm !== "" || this.isSearchActive) {
      this.searchTerm = "";
      this.isSearchActive = false;
      this.invalidateFilterCache(); // Clear cache since we're removing filter
      
      // Update context for conditional UI elements
      this.updateSearchContext();
      
      this._onDidChangeTreeData.fire(null);
    }
  }

  private updateSearchContext(): void {
    // Set context variables for conditional UI elements
    vscode.commands.executeCommand('setContext', 'sweetpad.builds.searchActive', this.isSearchActive);
    vscode.commands.executeCommand('setContext', 'sweetpad.builds.searchTerm', this.searchTerm || '');
  }

  private computeFilteredCache(): void {
    if (!this.isSearchActive) {
      // No search active, clear cached filtered data
      this.cachedFilteredWorkspaces = null;
      this.cachedFilteredRecentWorkspaces = null;
      return;
    }

    // Cache filtered workspaces
    this.cachedFilteredWorkspaces = this.filterWorkspaces(this.workspaces);
    this.cachedFilteredRecentWorkspaces = this.filterWorkspaces(this.recentWorkspaces);
  }

  public getSearchTerm(): string {
    return this.searchTerm;
  }

  public isSearching(): boolean {
    return this.isSearchActive;
  }

  // Filter workspaces based on search term
  private filterWorkspaces(workspaces: WorkspaceGroupTreeItem[]): WorkspaceGroupTreeItem[] {
    if (!this.isSearchActive) {
      return workspaces;
    }

    return workspaces.filter(workspace => {
      // Extract workspace name for comparison
      const workspaceName = workspace.label?.toString().toLowerCase() || "";
      const workspacePath = workspace.workspacePath.toLowerCase();
      
      // Check if workspace name or path contains search term
      return workspaceName.includes(this.searchTerm) || workspacePath.includes(this.searchTerm);
    });
  }

  // Filter schemes based on search term
  private async filterSchemes(schemes: BuildTreeItem[]): Promise<BuildTreeItem[]> {
    if (!this.isSearchActive) {
      return schemes;
    }

    return schemes.filter(scheme => {
      const schemeName = scheme.label?.toString().toLowerCase() || "";
      return schemeName.includes(this.searchTerm);
    });
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
    this.recentWorkspacesStorage = this.recentWorkspacesStorage.filter(path => path !== workspacePath);
    
    // Add to the front of the array
    this.recentWorkspacesStorage.unshift(workspacePath);
    
    // Keep only the most recent MAX_RECENT_ITEMS
    this.recentWorkspacesStorage = this.recentWorkspacesStorage.slice(0, this.MAX_RECENT_ITEMS);
    
    // Update our instance variable with tree items
    this.recentWorkspaces = this.recentWorkspacesStorage.map(path => {
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
  
  // Add a workspace to the tree and refresh the UI efficiently
  private addWorkspace(workspacePath: string): void {
    // First check if this workspace is already in the main list
    if (this.workspaces.some(w => w.workspacePath === workspacePath)) {
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
    
    // Sort workspaces by type and name
    this.sortWorkspaces();
    
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
  private addWorkspaceToSection(workspacePath: string, expectedSection: string): void {
    // First check if this workspace is already in the main list
    if (this.workspaces.some(w => w.workspacePath === workspacePath)) {
      return;
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
    
    // Sort workspaces by type and name
    this.sortWorkspaces();
    
    // Invalidate cache since workspace list changed
    this.invalidateFilterCache();
    
    // Trigger immediate UI refresh to show the new item in its section
    this._onDidChangeTreeData.fire(undefined);
    
    // If this is the current workspace, add it to recents
    if (isCurrentWorkspace) {
      this.addToRecentWorkspaces(workspacePath);
    }
  }
  
  // Sort workspaces by type (workspace, package, bazel) and then by name
  private sortWorkspaces(): void {
    this.workspaces.sort((a, b) => {
      // Define the category order
      const getCategory = (item: WorkspaceGroupTreeItem): number => {
        const path = item.workspacePath;
        if (path.endsWith(".xcworkspace") || path.includes(".xcworkspace/") ||
            path.endsWith(".xcodeproj") || path.includes(".xcodeproj/")) {
          return 1; // Workspaces (including projects) first
        } else if (path.endsWith("Package.swift")) {
          return 2; // Packages second
        } else if (path.endsWith("BUILD.bazel") || path.endsWith("BUILD")) {
          return 3; // Bazel third
        }
        return 4; // Other files last
      };
      
      // Get categories for comparison
      const catA = getCategory(a);
      const catB = getCategory(b);
      
      // First sort by category
      if (catA !== catB) {
        return catA - catB;
      }
      
      // Then sort alphabetically by display name
      // Extract display names from paths for better sorting
      const getDisplayName = (item: WorkspaceGroupTreeItem): string => {
        const filePath = item.workspacePath;
        
        if (path.basename(filePath) === "Package.swift") {
          return path.basename(path.dirname(filePath)).toLowerCase();
        }
        
        // For Xcode projects and workspaces, extract the project name
        const xcodeMatch = filePath.match(/([^/]+)\.(xcodeproj|xcworkspace)/);
        if (xcodeMatch) {
          return xcodeMatch[1].toLowerCase();
        }
        
        return path.basename(filePath).toLowerCase();
      };
      
      return getDisplayName(a).localeCompare(getDisplayName(b));
    });
  }

  // Helper to get the category of a workspace
  private getWorkspaceCategory(workspacePath: string): string {
    if (workspacePath.endsWith(".xcworkspace") || workspacePath.includes(".xcworkspace/") ||
        workspacePath.endsWith(".xcodeproj") || workspacePath.includes(".xcodeproj/")) {
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
    // Make sure workspaces are sorted
    this.sortWorkspaces();

    // Group by category
    const workspacesByCategory = new Map<string, WorkspaceGroupTreeItem[]>();
    
    // Use cached filtered data if available, otherwise compute
    let filteredRecentWorkspaces: WorkspaceGroupTreeItem[];
    let filteredWorkspaces: WorkspaceGroupTreeItem[];
    
    if (this.isSearchActive) {
      // Ensure filtered cache is computed
      if (this.cachedFilteredRecentWorkspaces === null || this.cachedFilteredWorkspaces === null) {
        this.computeFilteredCache();
      }
      filteredRecentWorkspaces = this.cachedFilteredRecentWorkspaces || [];
      filteredWorkspaces = this.cachedFilteredWorkspaces || [];
    } else {
      // No search active, use original data
      filteredRecentWorkspaces = this.recentWorkspaces;
      filteredWorkspaces = this.workspaces;
    }
    
    // Add recents section if we have any
    if (filteredRecentWorkspaces.length > 0) {
      workspacesByCategory.set("recent", [...filteredRecentWorkspaces]);
    }
    
    // Process regular workspaces
    for (const workspace of filteredWorkspaces) {
      const category = this.getWorkspaceCategory(workspace.workspacePath);
      
      if (!workspacesByCategory.has(category)) {
        workspacesByCategory.set(category, []);
      }
      
      workspacesByCategory.get(category)?.push(workspace);
    }
    
    // Create section items for each category
    const sections: WorkspaceSectionTreeItem[] = [];
    
    // Define the order we want categories to appear
    const categoryOrder = ["recent", "workspace", "package", "bazel", "other"];
    
    for (const category of categoryOrder) {
      const workspaces = workspacesByCategory.get(category) || [];
      let totalCount: number | undefined;
      
      // Show sections even if empty during loading (except "recent")
      const shouldShowEmptySection = (category !== "recent") && this.sectionsLoading.has(category);
      
      if (workspaces.length > 0 || shouldShowEmptySection) {
        // Calculate total count for filtered indicator
        if (this.isSearchActive) {
          if (category === "recent") {
            totalCount = this.recentWorkspaces.length;
          } else {
            // Count original workspaces in this category
            totalCount = this.workspaces.filter(w => this.getWorkspaceCategory(w.workspacePath) === category).length;
          }
        }
        
        const isLoading = this.sectionsLoading.has(category);
        sections.push(new WorkspaceSectionTreeItem(category, workspaces, this.searchTerm, totalCount, isLoading));
      }
    }
    
    return sections;
  }

  // Load workspaces with streaming updates to the UI
  private async loadWorkspacesStreamingly(): Promise<void> {
    if (this.isLoadingWorkspaces) {
      return; // Prevent multiple concurrent loading sessions
    }
    
    this.isLoadingWorkspaces = true;
    this.workspaces = []; // Reset workspaces list
    
    // Reset sections loading state
    this.sectionsLoading = new Set<string>(["workspace", "package", "bazel"]);
    this.subsectionLoadingCount = new Map<string, number>([
      ["workspace", 2], // xcworkspace + xcodeproj searches
      ["package", 1],   // Package.swift search  
      ["bazel", 1]      // BUILD.bazel search
    ]);
    
    try {
      // First check if we have cached workspaces from previous sessions
      if (this.context) {
        const cachedWorkspacePath = getCurrentXcodeWorkspacePath(this.context);
        if (cachedWorkspacePath) {
          this.addWorkspace(cachedWorkspacePath);
        }
      }
      
      // Start streaming search for workspaces without awaiting completion
      // Don't block UI while searching for workspaces
      void this.streamingWorkspaceSearch();
      
      // Update context variable for welcome screen
      void vscode.commands.executeCommand(
        "setContext", 
        "sweetpad.workspaces.noWorkspaces", 
        this.workspaces.length === 0
      );
    } catch (error) {
      commonLogger.error("Failed to load workspaces", { error });
      void vscode.commands.executeCommand("setContext", "sweetpad.workspaces.noWorkspaces", true);
      this.isLoadingWorkspaces = false;
    }
  }

  // Perform workspace search with parallel loading and immediate UI updates
  private async streamingWorkspaceSearch(): Promise<void> {
    const workspace = getWorkspacePath();
    
    try {
      // Immediately show empty sections with loading indicators
      this._onDidChangeTreeData.fire(undefined);
      
      console.log(`🚀 Starting parallel searches for sections:`, Array.from(this.sectionsLoading));
      console.log(`📊 Subsection counts:`, Object.fromEntries(this.subsectionLoadingCount));
      
      // Start all searches in parallel and track their completion
      const searchPromises = [
        // Search for Package.swift files (SPM packages)
        this.findFilesIncrementallyWithCallback({
          directory: workspace,
          depth: 4,
          matcher: (file) => file.name === "Package.swift",
          processFile: (filePath) => this.addWorkspaceToSection(filePath, "package"),
          onComplete: () => {
            console.log(`📦 Package search completed`);
            this.onSectionLoadComplete("package");
          }
        }),
        
        // Search for .xcworkspace files
        this.findFilesIncrementallyWithCallback({
          directory: workspace,
          depth: 4,
          matcher: (file) => file.name.endsWith("project.xcworkspace") || file.name.endsWith(".xcworkspace"),
          processFile: (filePath) => this.addWorkspaceToSection(filePath, "workspace"),
          onComplete: () => {
            console.log(`🏢 Workspace (.xcworkspace) search completed`);
            this.onSectionLoadComplete("workspace"); // Will decrement workspace counter
          }
        }),

        // Search for .xcodeproj files  
        this.findFilesIncrementallyWithCallback({
          directory: workspace,
          depth: 4,
          matcher: (file) => file.name.endsWith(".xcodeproj"),
          processFile: (filePath) => this.addWorkspaceToSection(filePath, "workspace"),
          onComplete: () => {
            console.log(`🏗️ Project (.xcodeproj) search completed`);
            this.onSectionLoadComplete("workspace"); // Will decrement workspace counter
          }
        }),
        
        // Search for Bazel BUILD files
        this.findFilesIncrementallyWithCallback({
          directory: workspace,
          depth: 4,
          matcher: (file) => file.name === "BUILD.bazel" || file.name === "BUILD",
          processFile: (filePath) => this.addWorkspaceToSection(filePath, "bazel"),
          onComplete: () => {
            console.log(`⚙️ Bazel search completed`);
            this.onSectionLoadComplete("bazel");
          }
        })
      ];
      
      // Wait for all searches to complete
      await Promise.all(searchPromises);
      
      // Mark overall loading as complete
      this.isLoadingWorkspaces = false;
      this._onDidChangeTreeData.fire(undefined);
      
      // Safety mechanism: Force complete any remaining sections after a timeout
      setTimeout(() => {
        if (this.sectionsLoading.size > 0) {
          console.warn(`⚠️ Forcing completion of stuck sections:`, Array.from(this.sectionsLoading));
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
    
    console.log(`🔍 Section ${sectionType} completed search: ${currentCount} -> ${newCount} remaining`);
    
    // Only mark section as complete when all its subsections are done
    if (newCount === 0) {
      this.sectionsLoading.delete(sectionType);
      console.log(`✅ Section ${sectionType} fully loaded, removing loading indicator`);
      // Trigger UI update to remove loading indicator for this section
      this._onDidChangeTreeData.fire(undefined);
    }
  }

  // Helper method to incrementally find and process files with completion callback
  private async findFilesIncrementallyWithCallback(options: {
    directory: string,
    matcher: (file: Dirent) => boolean,
    processFile: (filePath: string) => void,
    onComplete?: () => void,
    ignore?: string[],
    depth?: number
  }): Promise<void> {
    try {
      await this.findFilesIncrementally({
        directory: options.directory,
        matcher: options.matcher,
        processFile: options.processFile,
        ignore: options.ignore,
        depth: options.depth
      });
    } catch (error) {
      console.error(`🚫 Search failed for directory ${options.directory}:`, error);
    } finally {
      // Always call completion callback, even if search failed
      if (options.onComplete) {
        console.log(`🔄 Calling completion callback for ${options.directory}`);
        options.onComplete();
      }
    }
  }

  // Helper method to incrementally find and process files
  private async findFilesIncrementally(options: {
    directory: string,
    matcher: (file: Dirent) => boolean,
    processFile: (filePath: string) => void,
    ignore?: string[],
    depth?: number
  }): Promise<void> {
    const ignore = options.ignore ?? [];
    const depth = options.depth ?? 0;
    
    try {
      const files = await fs.readdir(options.directory, { withFileTypes: true });
      
      // Process matching files immediately
      for (const file of files) {
        const fullPath = path.join(options.directory, file.name);
        
        if (options.matcher(file)) {
          options.processFile(fullPath);
        }
        
        // Queue up directory searches to run in parallel
        if (file.isDirectory() && !ignore.includes(file.name) && depth > 0) {
          void this.findFilesIncrementally({
            directory: fullPath,
            matcher: options.matcher,
            processFile: options.processFile,
            ignore: options.ignore,
            depth: depth - 1
          });
        }
      }
    } catch (error) {
      commonLogger.error(`Error searching directory: ${options.directory}`, { error });
    }
  }

  async getChildren(element?: vscode.TreeItem): Promise<vscode.TreeItem[]> {
    // Start loading if needed
    if (this.workspaces.length === 0 && !this.isLoadingWorkspaces && !element) {
      this.loadWorkspacesStreamingly();
    }
    
    // Root level - show sections
    if (!element) {
      const sections = this.getSectionedWorkspaces();
      const results: vscode.TreeItem[] = [];
      
      // Add search status indicator at the top when filtering is active
      if (this.isSearchActive && this.searchTerm.length > 0) {
        const searchStatusItem = new vscode.TreeItem(`🔍 Filtering: "${this.searchTerm}"`, vscode.TreeItemCollapsibleState.None);
        searchStatusItem.iconPath = new vscode.ThemeIcon("search-stop");
        searchStatusItem.contextValue = "search-status";
        searchStatusItem.tooltip = `Click to clear search filter`;
        searchStatusItem.command = {
          command: 'sweetpad.build.clearSearch',
          title: 'Clear Search',
          arguments: []
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
        this.workspaces.length === 0
      );
      
      return results;
    }
    
    // For section items, return their workspaces
    if (element instanceof WorkspaceSectionTreeItem) {
      return element.workspaces;
    }

    // For workspace items, return schemes/targets ONLY if in Recents and for the specific workspace
    if (element instanceof WorkspaceGroupTreeItem) {
      // Only load schemes/targets for items in the Recents section
      if (element.isRecent) {
        // Check if this is a Bazel workspace
        if (element.workspacePath.endsWith("BUILD.bazel") || element.workspacePath.endsWith("BUILD")) {
          // Load Bazel targets for this workspace
          const buildFile = element.workspacePath;
          const bazelPackages = await getBazelPackages();
          const matchingPackage = bazelPackages.find(pkg => 
            pkg.path === path.dirname(buildFile)
          );
          
          if (matchingPackage) {
            // Create tree items for each target
            const targetItems = matchingPackage.targets.map(target => 
              new BazelTreeItem({
                target,
                package: matchingPackage,
                provider: this,
                workspacePath: element.workspacePath
              })
            );
            
            // Apply search filter if active
            if (this.isSearchActive && this.searchTerm.length > 0) {
              return targetItems.filter(item => 
                item.target.name.toLowerCase().includes(this.searchTerm)
              );
            }
            
            return targetItems;
          }
          
          return [];
        } else {
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
        }
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

export class BuildTreeItem extends vscode.TreeItem {
  public provider: WorkspaceTreeProvider;
  public scheme: string;
  public workspacePath: string;
  
  constructor(options: {
    scheme: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    provider: WorkspaceTreeProvider;
    workspacePath?: string;
  }) {
    super(options.scheme, options.collapsibleState);
    this.provider = options.provider;
    this.scheme = options.scheme;
    this.workspacePath = options.workspacePath || this.provider.defaultWorkspacePath || '';

    const color = new vscode.ThemeColor("sweetpad.scheme");
    this.iconPath = new vscode.ThemeIcon("sweetpad-package", color);
    this.contextValue = "sweetpad.build.view.item";

    let description = "";
    // Only show checkmark if this is the default scheme for this specific workspace
    if (this.scheme === this.provider.defaultSchemeForBuild && 
        this.workspacePath === this.provider.defaultWorkspacePath) {
      description = `${description} ✓`;
    }
    if (this.scheme === this.provider.defaultSchemeForTesting) {
      description = `${description} (t)`;
    }
    if (description) {
      this.description = description;
    }
    
    // Add workspace name to tooltip for clarity
    if (this.workspacePath) {
      const workspaceName = path.basename(this.workspacePath);
      this.tooltip = `Scheme: ${this.scheme}\nWorkspace: ${workspaceName}`;
      
      // Add workspace info to the label for clarity
      this.description = `${description || ''} (${workspaceName})`.trim();
    } else {
      this.tooltip = `Scheme: ${this.scheme}`;
    }
    
    // Store command with the correct arguments that point to this specific scheme and workspace
    this.command = {
      command: 'sweetpad.build.launch',
      title: 'Launch',
      arguments: [this]
    };
  }
}

export class BazelTreeItem extends vscode.TreeItem {
  public provider: WorkspaceTreeProvider;
  public target: BazelTarget;
  public package: BazelPackage;
  public workspacePath: string;
  
  constructor(options: {
    target: BazelTarget;
    package: BazelPackage;
    provider: WorkspaceTreeProvider;
    workspacePath: string;
  }) {
    super(options.target.name, vscode.TreeItemCollapsibleState.None);
    this.provider = options.provider;
    this.target = options.target;
    this.package = options.package;
    this.workspacePath = options.workspacePath;

    // Set icon based on target type
    const color = new vscode.ThemeColor("sweetpad.scheme");
    if (this.target.type === "test") {
      this.iconPath = new vscode.ThemeIcon("beaker", color);
    } else if (this.target.type === "library") {
      this.iconPath = new vscode.ThemeIcon("library", color);
    } else {
      this.iconPath = new vscode.ThemeIcon("gear", color);
    }
    
    this.contextValue = "sweetpad.bazel.target";
    
    // Add type and package info to description
    this.description = `${this.target.type} • ${this.package.name}`;
    
    // Set tooltip with build and test commands
    let tooltip = `Target: ${this.target.name}\nType: ${this.target.type}\nPackage: ${this.package.name}`;
    tooltip += `\nBuild: bazel build ${this.target.buildLabel}`;
    if (this.target.testLabel) {
      tooltip += `\nTest: bazel test ${this.target.testLabel}`;
    }
    this.tooltip = tooltip;
    
    // Set command to build the target
    this.command = {
      command: 'sweetpad.bazel.build',
      title: 'Build Bazel Target',
      arguments: [this]
    };
  }
}
