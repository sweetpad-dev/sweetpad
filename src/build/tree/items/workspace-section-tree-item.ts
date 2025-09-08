import * as vscode from "vscode";
import type { WorkspaceGroupTreeItem } from "./workspace-group-tree-item";

// Section header for grouping workspaces
export class WorkspaceSectionTreeItem extends vscode.TreeItem {
  public readonly workspaces: WorkspaceGroupTreeItem[];
  public readonly sectionType: string;
  public readonly isLoading: boolean;

  constructor(
    sectionType: string,
    workspaces: WorkspaceGroupTreeItem[],
    searchTerm?: string,
    totalCount?: number,
    isLoading?: boolean,
  ) {
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
      if (workspaces.length > 0 && workspaces[0]?.provider?.context?.extensionPath) {
        this.iconPath = vscode.Uri.joinPath(
          vscode.Uri.file(workspaces[0].provider.context.extensionPath),
          "images",
          "xcodeproj.png",
        );
      } else {
        this.iconPath = new vscode.ThemeIcon("multiple-windows");
      }
    } else if (sectionType === "package") {
      if (workspaces.length > 0 && workspaces[0]?.provider?.context?.extensionPath) {
        this.iconPath = vscode.Uri.joinPath(
          vscode.Uri.file(workspaces[0].provider.context.extensionPath),
          "images",
          "spm.png",
        );
      } else {
        this.iconPath = new vscode.ThemeIcon("extensions");
      }
    } else if (sectionType === "bazel") {
      // Use bazel.png icon from images folder, fallback to gear icon if no workspaces
      if (workspaces.length > 0 && workspaces[0]?.provider?.context?.extensionPath) {
        this.iconPath = vscode.Uri.joinPath(
          vscode.Uri.file(workspaces[0].provider.context.extensionPath),
          "images",
          "bazel.png",
        );
      } else {
        this.iconPath = new vscode.ThemeIcon("gear");
      }
    } else {
      this.iconPath = new vscode.ThemeIcon("folder");
    }
  }
}
