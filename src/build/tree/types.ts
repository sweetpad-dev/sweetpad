import type { WorkspaceGroupTreeItem } from "./items/workspace-group-tree-item";

// Type definitions for tree events
export type WorkspaceEventData = WorkspaceGroupTreeItem | undefined | null;
export type BuildEventData = any | undefined | null; // BuildTreeItem will be defined in build-tree-item.ts

// Persistent cache interface
export interface WorkspaceCacheData {
  version: string;
  timestamp: number;
  workspacePaths: string[];
  recentWorkspacePaths: string[];
  workspaceRoot: string;
}
