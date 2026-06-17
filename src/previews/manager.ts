import { promises as fs } from "node:fs";

import * as vscode from "vscode";

import { commonLogger } from "../common/logger.js";
import { parsePreviews, previewId } from "./parser.js";
import type { PreviewItem } from "./types.js";

// Directories that hold build output or vendored code — never worth scanning
// for the user's previews, and expensive to walk.
const EXCLUDE_GLOB = "**/{.build,.git,DerivedData,Pods,Carthage,node_modules,.swiftpm}/**";

/**
 * Indexes the SwiftUI previews in the workspace and keeps the index fresh as
 * `.swift` files change. Backs the previews tree view and the "render preview"
 * quick pick. Pure discovery — rendering/streaming lives in {@link PreviewHostManager}.
 */
export class PreviewsManager implements vscode.Disposable {
  /** relativePath -> previews in that file, in source order. */
  private index = new Map<string, PreviewItem[]>();
  private watcher: vscode.FileSystemWatcher | undefined;
  private scanned = false;

  private onDidChangeEmitter = new vscode.EventEmitter<void>();
  /** Fires whenever the index changes (a file was scanned, edited, or removed). */
  readonly onDidChange = this.onDidChangeEmitter.event;

  start(): void {
    this.watcher = vscode.workspace.createFileSystemWatcher("**/*.swift");
    this.watcher.onDidCreate((uri) => void this.scanFile(uri));
    this.watcher.onDidChange((uri) => void this.scanFile(uri));
    this.watcher.onDidDelete((uri) => this.forgetFile(uri));
  }

  dispose(): void {
    this.watcher?.dispose();
    this.onDidChangeEmitter.dispose();
    this.index.clear();
  }

  /** All discovered previews, grouped by file and sorted by path. */
  async getGrouped(): Promise<{ relativePath: string; uri: vscode.Uri; previews: PreviewItem[] }[]> {
    await this.ensureScanned();
    return [...this.index.entries()]
      .filter(([, previews]) => previews.length > 0)
      .toSorted(([a], [b]) => a.localeCompare(b))
      .map(([relativePath, previews]) => ({
        relativePath: relativePath,
        uri: previews[0].uri,
        previews: previews,
      }));
  }

  /** Flat list of all previews, for the quick pick. */
  async getAll(): Promise<PreviewItem[]> {
    await this.ensureScanned();
    return [...this.index.values()].flat();
  }

  async refresh(): Promise<void> {
    this.index.clear();
    this.scanned = false;
    await this.ensureScanned();
  }

  /**
   * Build {@link PreviewItem}s for a single document's text. Shared with the
   * CodeLens provider so the lens and the index agree on locations.
   */
  itemsForDocument(uri: vscode.Uri, text: string): PreviewItem[] {
    const relativePath = this.toRelative(uri);
    return parsePreviews(text).map((match) => ({
      id: previewId(relativePath, match),
      kind: match.kind,
      label: match.label ?? (match.kind === "provider" ? "PreviewProvider" : "Preview"),
      uri: uri,
      relativePath: relativePath,
      match: match,
    }));
  }

  private async ensureScanned(): Promise<void> {
    if (this.scanned) return;
    this.scanned = true;
    try {
      const files = await vscode.workspace.findFiles("**/*.swift", EXCLUDE_GLOB);
      await Promise.all(files.map((uri) => this.scanFile(uri, { silent: true })));
    } catch (error) {
      commonLogger.error("Failed to scan workspace for SwiftUI previews", { error: error });
    }
    this.onDidChangeEmitter.fire();
  }

  private async scanFile(uri: vscode.Uri, options?: { silent?: boolean }): Promise<void> {
    let text: string;
    try {
      text = await fs.readFile(uri.fsPath, "utf-8");
    } catch {
      // File vanished between discovery and read — treat as removed.
      this.forgetFile(uri);
      return;
    }
    const items = this.itemsForDocument(uri, text);
    const key = this.toRelative(uri);
    if (items.length > 0) {
      this.index.set(key, items);
    } else {
      this.index.delete(key);
    }
    if (!options?.silent) {
      this.onDidChangeEmitter.fire();
    }
  }

  private forgetFile(uri: vscode.Uri): void {
    if (this.index.delete(this.toRelative(uri))) {
      this.onDidChangeEmitter.fire();
    }
  }

  private toRelative(uri: vscode.Uri): string {
    return vscode.workspace.asRelativePath(uri, false);
  }
}
