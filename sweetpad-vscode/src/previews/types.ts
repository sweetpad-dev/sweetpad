import type * as vscode from "vscode";

import type { PreviewKind, PreviewMatch } from "./parser.js";

/**
 * A single SwiftUI preview discovered in the workspace, enriched with its
 * location so it can be opened in the editor or rendered by the preview host.
 */
export interface PreviewItem {
  /** Stable `<relativePath>:<line>` identifier passed to the preview host. */
  id: string;
  kind: PreviewKind;
  /** Display label: the macro's string argument, the provider type, or a fallback. */
  label: string;
  uri: vscode.Uri;
  /** Workspace-relative POSIX path, e.g. `Sources/Feature/ContentView.swift`. */
  relativePath: string;
  match: PreviewMatch;
}
