import * as path from "node:path";
import * as vscode from "vscode";
import type { XcodeScheme } from "../common/cli/scripts";
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

  constructor(options: { workspacePath: string; provider: WorkspaceTreeProvider; isRecent?: boolean }) {
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

    // What constructs the display of the tree item
    super(displayName, vscode.TreeItemCollapsibleState.Expanded);

    this.workspacePath = options.workspacePath;
    this.provider = options.provider;
    this.isRecent = !!options.isRecent;

    let description = "";
    let color: vscode.ThemeColor;

    if (this.workspacePath === this.provider.defaultWorkspacePath) {
      description = `${description} ✓`;
      color = new vscode.ThemeColor("sweetpad.workspace");
      this.collapsibleState = vscode.TreeItemCollapsibleState.Expanded; // if the workspace is the current workspace, show it expanded
    } else {
      color = new vscode.ThemeColor("foreground");
      this.collapsibleState = vscode.TreeItemCollapsibleState.None; // if the workspace is not the current workspace, show it collapsed
    }

    if (description) {
      this.description = description;
    }
    
    // Set icon based on file type
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
      this.buildManager.refresh();
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

  private refresh(): void {
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
    this.recentWorkspaces = this.recentWorkspacesStorage.map(path => 
      new WorkspaceGroupTreeItem({
        workspacePath: path,
        provider: this,
        isRecent: true
      })
    );
  }

  // Add a workspace to the tree and refresh the UI immediately
  private addWorkspace(workspacePath: string): void {
    // Check for duplicates - don't add if already exists
    if (this.workspaces.some(w => w.workspacePath === workspacePath)) {
      return;
    }
    
    const workspaceItem = new WorkspaceGroupTreeItem({
      workspacePath: workspacePath,
      provider: this,
    });
    
    this.workspaces.push(workspaceItem);
    
    // Sort workspaces by type and name
    this.sortWorkspaces();
    
    // Notify UI to refresh
    this._onDidChangeTreeData.fire(undefined);
    
    // If this is the current workspace, add it to recents
    if (workspacePath === this.defaultWorkspacePath) {
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
    
    // Add recents section if we have any
    if (this.recentWorkspaces.length > 0) {
      workspacesByCategory.set("recent", [...this.recentWorkspaces]);
    }
    
    // Process regular workspaces
    for (const workspace of this.workspaces) {
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
      this.streamingWorkspaceSearch();
      
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
        results.push(new vscode.TreeItem("Searching for more workspaces...", vscode.TreeItemCollapsibleState.None));
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

    // For workspace items, return schemes
    if (element instanceof WorkspaceGroupTreeItem) {
      return await this.getSchemes();
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

  async getSchemes(): Promise<BuildTreeItem[]> {
    let schemes: XcodeScheme[] = [];
    try {
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

    // return list of schemes
    return schemes.map(
      (scheme) =>
        new BuildTreeItem({
          scheme: scheme.name,
          collapsibleState: vscode.TreeItemCollapsibleState.None,
          provider: this,
        }),
    );
  }
}

export class BuildTreeItem extends vscode.TreeItem {
  public provider: WorkspaceTreeProvider;
  public scheme: string;

  constructor(options: {
    scheme: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    provider: WorkspaceTreeProvider;
  }) {
    super(options.scheme, options.collapsibleState);
    this.provider = options.provider;
    this.scheme = options.scheme;

    const color = new vscode.ThemeColor("sweetpad.scheme");
    this.iconPath = new vscode.ThemeIcon("sweetpad-package", color);
    this.contextValue = "sweetpad.build.view.item";

    let description = "";
    if (this.scheme === this.provider.defaultSchemeForBuild) {
      description = `${description} ✓`;
    }
    if (this.scheme === this.provider.defaultSchemeForTesting) {
      description = `${description} (t)`;
    }
    if (description) {
      this.description = description;
    }
  }
}
