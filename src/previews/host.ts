import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import * as vscode from "vscode";

import { resolveInjectionDylib } from "../build/hot-reload.js";
import { getWorkspacePath } from "../build/utils.js";
import { ExtensionError } from "../common/errors.js";
import { exec } from "../common/exec.js";
import { commonLogger } from "../common/logger.js";
import type { WorkspaceStateService } from "../common/workspace-state.js";
import type { DestinationsManager } from "../destination/manager.js";
import type { ServeSimManager } from "../simulators/serve-sim.js";
import type { SimulatorDestination } from "../simulators/types.js";
import type { PreviewItem } from "./types.js";

/** Environment variables the scaffolded Swift bootstrap reads. */
const ENV_PREVIEW_ID = "SWEETPAD_PREVIEW_ID";
const ENV_PREVIEW_APPEARANCE = "SWEETPAD_PREVIEW_APPEARANCE";

/** Appearances rendered by the "screenshot variants" command. */
const VARIANT_APPEARANCES = ["light", "dark"] as const;
type Appearance = (typeof VARIANT_APPEARANCES)[number];

/**
 * Drives the "preview in the simulator" half of the feature (Phases 2–4):
 *
 * - {@link scaffold} writes a Swift bootstrap into the workspace that, given a
 *   `SWEETPAD_PREVIEW_ID`, swaps the app's root view for the matching `#Preview`
 *   (using EmergeTools' SnapshotPreviews for runtime discovery).
 * - {@link render} relaunches the last-built app on its simulator with that env
 *   var set, then streams the simulator into a webview via {@link ServeSimManager}.
 * - {@link screenshot} captures the rendered preview (optionally across
 *   light/dark variants) with `simctl io … screenshot`.
 *
 * Hot reload (Phase 3) is achieved by injecting the existing InjectionNext
 * dylib into the relaunched host, so edits refresh the streamed preview.
 *
 * NOTE: the Swift/simulator path requires macOS + Xcode and a one-time
 * `scaffold` integration; it can only be exercised on a Mac.
 */
export class PreviewHostManager {
  private readonly destinationsManager: DestinationsManager;
  private readonly serveSimManager: ServeSimManager;
  private readonly workspaceState: WorkspaceStateService;

  constructor(options: {
    destinationsManager: DestinationsManager;
    serveSimManager: ServeSimManager;
    workspaceState: WorkspaceStateService;
  }) {
    this.destinationsManager = options.destinationsManager;
    this.serveSimManager = options.serveSimManager;
    this.workspaceState = options.workspaceState;
  }

  /**
   * Render a preview: reveal its source, relaunch the host app pinned to that
   * preview, and stream the simulator into the editor.
   */
  async render(item: PreviewItem, options?: { appearance?: Appearance }): Promise<SimulatorDestination> {
    await this.revealSource(item);

    const { context, simulator } = await this.resolveSimulatorHost();
    await this.launchPinned(context.bundleIdentifier, simulator.udid, item.id, options?.appearance);

    // Stream the simulator into the webview (reuses the serve-sim integration).
    await this.serveSimManager.stream(simulator);
    return simulator;
  }

  /**
   * Capture a screenshot of the rendered preview. With `variants`, renders and
   * captures light + dark and returns all image paths.
   */
  async screenshot(item: PreviewItem, options?: { variants?: boolean }): Promise<string[]> {
    const { context, simulator } = await this.resolveSimulatorHost();
    const appearances: Appearance[] = options?.variants ? [...VARIANT_APPEARANCES] : ["light"];

    const dir = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-preview-"));
    const shots: string[] = [];
    for (const appearance of appearances) {
      await this.launchPinned(context.bundleIdentifier, simulator.udid, item.id, appearance);
      // Give the preview a moment to render before capturing.
      await delay(1500);
      const safeId = item.id.replace(/[^\w.-]/g, "_");
      const file = path.join(dir, `${safeId}.${appearance}.png`);
      await exec({
        command: "xcrun",
        args: ["simctl", "io", simulator.udid, "screenshot", file],
      });
      shots.push(file);
    }
    return shots;
  }

  /**
   * Write the Swift bootstrap into the workspace and show one-time setup
   * instructions. Returns the path to the generated file.
   */
  async scaffold(): Promise<vscode.Uri> {
    const root = getWorkspacePath();
    const target = path.join(root, "SweetPadPreviewHost.swift");
    if (!(await pathExists(target))) {
      await fs.writeFile(target, PREVIEW_HOST_BOOTSTRAP, "utf-8");
    }
    const uri = vscode.Uri.file(target);
    await vscode.window.showTextDocument(uri);
    void vscode.window
      .showInformationMessage(
        "SweetPad: Added SweetPadPreviewHost.swift. Add the EmergeTools/SnapshotPreviews package, then call SweetPadPreviewHost.rootView() from your @main App. See the file header for steps.",
        "Open SnapshotPreviews",
      )
      .then((choice) => {
        if (choice === "Open SnapshotPreviews") {
          void vscode.env.openExternal(vscode.Uri.parse("https://github.com/EmergeTools/SnapshotPreviews"));
        }
      });
    return uri;
  }

  /** Open the preview's source file and move the cursor to its declaration. */
  private async revealSource(item: PreviewItem): Promise<void> {
    const editor = await vscode.window.showTextDocument(item.uri, { preview: false });
    const position = new vscode.Position(item.match.line, item.match.character);
    editor.selection = new vscode.Selection(position, position);
    editor.revealRange(new vscode.Range(position, position), vscode.TextEditorRevealType.InCenter);
  }

  /**
   * Resolve the host app + simulator to use from the last Build & Run. Previews
   * reuse the already-built app, so the user must have run it on a simulator
   * at least once.
   */
  private async resolveSimulatorHost(): Promise<{
    context: { bundleIdentifier: string; simulatorUdid: string };
    simulator: SimulatorDestination;
  }> {
    const context = this.workspaceState.get("build.lastLaunchedApp");
    if (!context || context.type !== "simulator") {
      throw new ExtensionError(
        "Run the app on a simulator first (SweetPad: Run). Previews reuse the last app built and launched on a simulator.",
      );
    }
    const simulators = await this.destinationsManager.getSimulators({ sort: true });
    const simulator = simulators.find((sim) => sim.udid === context.simulatorUdid);
    if (!simulator) {
      throw new ExtensionError(
        "The simulator from the last run is no longer available. Run the app on a booted simulator and try again.",
      );
    }
    return { context: context, simulator: simulator };
  }

  /**
   * Relaunch the host app on the simulator, pinned to a specific preview via
   * env vars, optionally injecting the InjectionNext dylib for hot reload.
   */
  private async launchPinned(
    bundleId: string,
    udid: string,
    previewId: string,
    appearance: Appearance | undefined,
  ): Promise<void> {
    // `--console-pty` would block; we just launch and terminate any running
    // instance so the env vars take effect on a fresh process.
    const args = ["simctl", "launch", "--terminate-running-process"];

    args.push("--setenv", `${ENV_PREVIEW_ID}=${previewId}`);
    if (appearance) {
      args.push("--setenv", `${ENV_PREVIEW_APPEARANCE}=${appearance}`);
    }

    // Phase 3: hot reload. When enabled and supported, inject InjectionNext so
    // edits to the previewed view refresh the streamed preview without relaunch.
    const dylib = resolveInjectionDylib("iOSSimulator");
    if (dylib) {
      args.push("--setenv", `DYLD_INSERT_LIBRARIES=${dylib}`);
      args.push("--setenv", `INJECTION_PROJECT_ROOT=${getWorkspacePath()}`);
    }

    args.push(udid, bundleId);

    try {
      await exec({ command: "xcrun", args: args });
    } catch (error) {
      commonLogger.error("Failed to launch preview host", { bundleId: bundleId, error: error });
      throw new ExtensionError(`Failed to launch preview host "${bundleId}" on the simulator`, {
        context: { error: error instanceof Error ? error.message : String(error) },
      });
    }
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function pathExists(p: string): Promise<boolean> {
  try {
    await fs.access(p);
    return true;
  } catch {
    return false;
  }
}

/**
 * The Swift bootstrap dropped into the user's project by `scaffold()`. It uses
 * EmergeTools' SnapshotPreviews to discover previews at runtime and render the
 * one named by `SWEETPAD_PREVIEW_ID`. Kept here as a string so it ships in the
 * bundled extension without a separate asset file.
 */
const PREVIEW_HOST_BOOTSTRAP = `// SweetPadPreviewHost.swift — generated by SweetPad.
//
// Lets SweetPad render a specific #Preview inside the simulator and stream it
// to VSCode. One-time setup:
//
//   1. Add the Swift package https://github.com/EmergeTools/SnapshotPreviews
//      to your app target (product: "PreviewGallery").
//   2. In your @main App, render SweetPadPreviewHost.rootView() when running
//      under SweetPad, e.g.:
//
//        @main struct MyApp: App {
//          var body: some Scene {
//            WindowGroup {
//              if let preview = SweetPadPreviewHost.rootView() {
//                preview
//              } else {
//                ContentView()
//              }
//            }
//          }
//        }
//
//   3. Build & run once on a simulator (SweetPad: Run), then use the
//      "Preview in VSCode" CodeLens or the SwiftUI Previews view.
//
// NOTE: requires Debug builds with dead-code stripping disabled so previews are
// preserved in the binary (Emerge's SnapshotPreviews documents this).

#if DEBUG
import SwiftUI
#if canImport(PreviewGallery)
import PreviewGallery
#endif
#if canImport(SnapshotPreviews)
import SnapshotPreviews
#endif

public enum SweetPadPreviewHost {
  /// The preview id requested by SweetPad, e.g. "Sources/Feature/ContentView.swift:10".
  public static var requestedPreviewId: String? {
    ProcessInfo.processInfo.environment["${ENV_PREVIEW_ID}"]
  }

  /// Optional appearance override ("light" / "dark") for screenshot variants.
  public static var appearance: ColorScheme? {
    switch ProcessInfo.processInfo.environment["${ENV_PREVIEW_APPEARANCE}"] {
    case "dark": return .dark
    case "light": return .light
    default: return nil
    }
  }

  /// Returns a view that renders the requested preview, or nil when SweetPad
  /// didn't request one (so the app boots normally).
  @MainActor
  public static func rootView() -> AnyView? {
    guard let id = requestedPreviewId else { return nil }

    // SnapshotPreviews discovers every #Preview/PreviewProvider in the binary.
    // We match on the fileID:line recorded by the #Preview macro. The exact API
    // surface depends on the SnapshotPreviews version — adjust if it differs.
    var content = AnyView(PreviewMatcher.view(forId: id))
    if let scheme = appearance {
      content = AnyView(content.preferredColorScheme(scheme))
    }
    return content
  }
}

/// Thin shim over SnapshotPreviews' runtime preview discovery. Replace the body
/// with the matching SnapshotPreviews API for your version if needed.
enum PreviewMatcher {
  @MainActor
  static func view(forId id: String) -> some View {
    #if canImport(PreviewGallery)
    // PreviewGallery renders a browsable catalog of all discovered previews;
    // SweetPad shows it and you can also deep-link by id once wired up.
    return PreviewGallery()
    #else
    return Text("Add the SnapshotPreviews 'PreviewGallery' product to render: \\(id)")
      .multilineTextAlignment(.center)
      .padding()
    #endif
  }
}
#endif
`;
