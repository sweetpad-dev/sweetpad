import path from "node:path";
import * as vscode from "vscode";
import type { XcodeScheme } from "../common/cli/scripts";
import { getSchemes } from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { commonLogger } from "../common/logger";
import type { BuildManager } from "./manager";
import { getCurrentXcodeWorkspacePath, getWorkspacePath } from "./utils";
import { detectXcodeWorkspacesPaths } from "./utils";
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
    
    // Set icon based on file type - keep original icon system
    if (this.workspacePath.includes("DoorDash.xcodeproj")) {
      // Special case for DoorDash xcodeproj
      this.iconPath = vscode.Uri.joinPath(vscode.Uri.file(this.provider.context?.extensionPath || ""), "images", "bazel.png");
    } 
    // Use Swift Package Manager icon for Package.swift files
    else if (path.basename(this.workspacePath) === "Package.swift") {
      this.iconPath = vscode.Uri.joinPath(vscode.Uri.file(this.provider.context?.extensionPath || ""), "images", "spm.png");
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
  
  constructor(sectionType: string, workspaces: WorkspaceGroupTreeItem[]) {
    // Format the section title with first letter capitalized and make it plural
    const label = sectionType.charAt(0).toUpperCase() + sectionType.slice(1) + (sectionType === "recent" ? "s" : "s");
    
    // Make sections collapsible and expanded by default
    super(label, vscode.TreeItemCollapsibleState.Expanded);
    
    this.workspaces = workspaces;
    this.sectionType = sectionType;
    this.contextValue = "workspace-section";
    
    // Use an appropriate icon based on section type
    if (sectionType === "recent") {
      this.iconPath = new vscode.ThemeIcon("history");
    } else if (sectionType === "workspace") {
      this.iconPath = new vscode.ThemeIcon("multiple-windows");
    } else if (sectionType === "project") {
      this.iconPath = new vscode.ThemeIcon("package");
    } else if (sectionType === "package") {
      this.iconPath = new vscode.ThemeIcon("extensions");
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
      this._onDidChangeTreeData.fire(null);
    }
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

  // Add a workspace to the tree and refresh the UI immediately
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
    
    // Notify UI to refresh
    this._onDidChangeTreeData.fire(undefined);
    
    // If this is the current workspace, add it to recents
    if (isCurrentWorkspace) {
      this.addToRecentWorkspaces(workspacePath);
    }
  }
  
  // Sort workspaces by type (workspace, project, package) and then by name
  private sortWorkspaces(): void {
    this.workspaces.sort((a, b) => {
      // Define the category order
      const getCategory = (item: WorkspaceGroupTreeItem): number => {
        const path = item.workspacePath;
        if (path.endsWith(".xcworkspace") || path.includes(".xcworkspace/")) {
          return 1; // Workspaces first
        } else if (path.endsWith(".xcodeproj") || path.includes(".xcodeproj/")) {
          return 2; // Projects second
        } else if (path.endsWith("Package.swift")) {
          return 3; // Packages last
        }
        return 4; // Other files (should not happen)
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
    if (workspacePath.endsWith(".xcworkspace") || workspacePath.includes(".xcworkspace/")) {
      return "workspace";
    } else if (workspacePath.endsWith(".xcodeproj") || workspacePath.includes(".xcodeproj/")) {
      return "project";
    } else if (workspacePath.endsWith("Package.swift")) {
      return "package";
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
    const categoryOrder = ["recent", "workspace", "project", "package", "other"];
    
    for (const category of categoryOrder) {
      const workspaces = workspacesByCategory.get(category);
      if (workspaces && workspaces.length > 0) {
        sections.push(new WorkspaceSectionTreeItem(category, workspaces));
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

  // Perform workspace search with incremental updates
  private async streamingWorkspaceSearch(): Promise<void> {
    const workspace = getWorkspacePath();
    
    try {
      // Start searching for Package.swift files
      this.findFilesIncrementally({
        directory: workspace,
        depth: 4,
        matcher: (file) => file.name === "Package.swift",
        processFile: (filePath) => this.addWorkspace(filePath)
      });
      
      // Start searching for xcworkspace files
      this.findFilesIncrementally({
        directory: workspace,
        depth: 4,
        matcher: (file) => file.name.endsWith("project.xcworkspace"),
        processFile: (filePath) => this.addWorkspace(filePath)
      });
      
      // Set a timeout to mark loading as complete after a reasonable time
      // This ensures the loading indicator eventually disappears even if some
      // directory operations are slow or fail silently
      setTimeout(() => {
        if (this.isLoadingWorkspaces) {
          this.isLoadingWorkspaces = false;
          this._onDidChangeTreeData.fire(undefined);
        }
      }, 10000); // 10 seconds timeout
    } catch (error) {
      commonLogger.error("Error in workspace search", { error });
      this.isLoadingWorkspaces = false;
      this._onDidChangeTreeData.fire(undefined);
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
      
      // Add a loading indicator if we're still searching
      const results: vscode.TreeItem[] = [...sections];
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

    // For workspace items, return schemes ONLY if in Recents and for the specific workspace
    if (element instanceof WorkspaceGroupTreeItem) {
      // Only load schemes for items in the Recents section
      if (element.isRecent) {
        // Check if we have cached schemes for this workspace + search term combination
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
