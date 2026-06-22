import * as vscode from "vscode";

import type { AppDeps } from "../common/commands.js";
import { showQuickPick } from "../common/quick-pick.js";
import type { PreviewItem } from "./types.js";

/**
 * Render a preview in VSCode. Invoked from the CodeLens, the previews tree, or
 * (without an item) the command palette — which then prompts for a preview.
 */
export async function renderPreviewCommand(deps: AppDeps, item?: PreviewItem) {
  const preview = item ?? (await pickPreview(deps, "Select a preview to render"));
  if (!preview) return;
  deps.progressStatusBar.updateText(`Rendering preview ${preview.label}`);
  await deps.previewHostManager.render(preview);
}

/** Re-scan the workspace for SwiftUI previews. */
export async function refreshPreviewsCommand(deps: AppDeps) {
  await deps.previewsManager.refresh();
}

/** One-time setup: scaffold the Swift bootstrap into the project. */
export async function setupPreviewHostCommand(deps: AppDeps) {
  await deps.previewHostManager.scaffold();
}

/** Capture a screenshot of the rendered preview. */
export async function screenshotPreviewCommand(deps: AppDeps, item?: PreviewItem) {
  const preview = item ?? (await pickPreview(deps, "Select a preview to screenshot"));
  if (!preview) return;
  deps.progressStatusBar.updateText(`Capturing preview ${preview.label}`);
  const shots = await deps.previewHostManager.screenshot(preview);
  await openImages(shots);
}

/** Capture light + dark screenshots of the rendered preview. */
export async function screenshotPreviewVariantsCommand(deps: AppDeps, item?: PreviewItem) {
  const preview = item ?? (await pickPreview(deps, "Select a preview to screenshot"));
  if (!preview) return;
  deps.progressStatusBar.updateText(`Capturing variants of ${preview.label}`);
  const shots = await deps.previewHostManager.screenshot(preview, { variants: true });
  await openImages(shots);
}

async function pickPreview(deps: AppDeps, title: string): Promise<PreviewItem | undefined> {
  const previews = await deps.previewsManager.getAll();
  if (previews.length === 0) {
    void vscode.window.showInformationMessage("SweetPad: No SwiftUI previews found in this workspace.");
    return undefined;
  }
  const selected = await showQuickPick({
    title: title,
    items: previews.map((preview) => ({
      label: preview.label,
      detail: preview.id,
      context: { preview: preview },
    })),
  });
  return selected.context.preview;
}

async function openImages(paths: string[]): Promise<void> {
  for (const p of paths) {
    await vscode.commands.executeCommand("vscode.open", vscode.Uri.file(p), {
      viewColumn: vscode.ViewColumn.Beside,
      preserveFocus: true,
    });
  }
}
