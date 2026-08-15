/**
 * Shared plumbing for the end-to-end suite. These tests run *inside* a real VS
 * Code extension host (see `.vscode-test.mjs`), so `vscode` here is the genuine
 * API rather than the unit tests' stub — the point of the layer is to exercise
 * activation, command registration and settings for real.
 */
import * as fs from "node:fs";
import * as path from "node:path";

import * as vscode from "vscode";

export const EXTENSION_ID = "sweetpad.sweetpad";

/** The scheme and container the fixture workspace provides. */
export const FIXTURE_SCHEME = "ObjCHeaders";
export const FIXTURE_PROJECT = "ObjCHeaders.xcodeproj";
/**
 * What `sweetpad.build.xcodeWorkspacePath` wants: a *workspace*. A plain project
 * is addressed through the one embedded in it, which is what the extension's own
 * picker stores — pointing the setting at the bare `.xcodeproj` makes every
 * build fail with "is not a workspace file".
 */
export const FIXTURE_CONTAINER = `${FIXTURE_PROJECT}/project.xcworkspace`;

/** The throwaway workspace folder the harness copied the fixture into. */
export function workspaceRoot(): string {
  const folder = vscode.workspace.workspaceFolders?.[0];
  if (!folder) {
    throw new Error("no workspace folder — the harness must open the fixture project");
  }
  return folder.uri.fsPath;
}

export function workspacePath(...segments: string[]): string {
  return path.join(workspaceRoot(), ...segments);
}

/**
 * The activated extension. Activation is normally triggered by the fixture's
 * `.xcodeproj`, but a test must not race that, so wait for it explicitly.
 */
export async function activate(): Promise<vscode.Extension<unknown>> {
  const extension = vscode.extensions.getExtension(EXTENSION_ID);
  if (!extension) {
    throw new Error(`${EXTENSION_ID} is not installed in the test instance`);
  }
  if (!extension.isActive) {
    await extension.activate();
  }
  return extension;
}

/**
 * Point the extension at the fixture's project and scheme through settings.
 *
 * Every "ask" helper reads its answer from configuration before falling back to
 * a QuickPick, so seeding these two keys is what keeps the suite non-interactive
 * — an unseeded run would hang on a prompt nobody can answer.
 */
export async function seedProjectSelection(): Promise<void> {
  const config = vscode.workspace.getConfiguration("sweetpad");
  await config.update(
    "build.xcodeWorkspacePath",
    workspacePath(FIXTURE_CONTAINER),
    vscode.ConfigurationTarget.Workspace,
  );
  await config.update("build.scheme", FIXTURE_SCHEME, vscode.ConfigurationTarget.Workspace);
}

/** Where a seeded build puts its output — chosen by us, so tests can find it. */
export function derivedDataPath(): string {
  return workspacePath(".derived");
}

/**
 * Everything a build needs in order to run without asking a question: the
 * project and scheme, plus the configuration, destination and output directory.
 * "My Mac" is the name the extension gives the local machine.
 */
export async function seedBuildSelection(): Promise<void> {
  await seedProjectSelection();
  const config = vscode.workspace.getConfiguration("sweetpad");
  await config.update("build.configuration", "Debug", vscode.ConfigurationTarget.Workspace);
  await config.update("build.derivedDataPath", derivedDataPath(), vscode.ConfigurationTarget.Workspace);
  await config.update("build.destination", { type: "macOS", id: "My Mac" }, vscode.ConfigurationTarget.Workspace);
  // Regenerating `buildServer.json` and restarting the language server on every
  // build is real behaviour, but it is the code-intelligence suite's subject,
  // not this one's. Turning it off here buys back seconds per build and stops a
  // build assertion failing for a reason that has nothing to do with building.
  await config.update("build.autoGenerateBuildServerConfig", false, vscode.ConfigurationTarget.Workspace);
  await config.update("build.autoRestartSwiftLSP", false, vscode.ConfigurationTarget.Workspace);
}

/**
 * Find a file by name anywhere under `root`, or undefined.
 *
 * Products are located by searching rather than by rebuilding Xcode's
 * `Build/Products/<configuration>/` layout in the assertion: where a product
 * lands inside the directory we chose is the build system's business, and this
 * suite must not care which build system that is.
 */
export function findFiles(root: string, name: string): string[] {
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(root, { withFileTypes: true });
  } catch {
    return [];
  }
  const found: string[] = [];
  for (const entry of entries) {
    const candidate = path.join(root, entry.name);
    if (entry.isDirectory()) {
      found.push(...findFiles(candidate, name));
    } else if (entry.name === name) {
      found.push(candidate);
    }
  }
  return found;
}

export function findFile(root: string, name: string): string | undefined {
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(root, { withFileTypes: true });
  } catch {
    return undefined;
  }
  for (const entry of entries) {
    const candidate = path.join(root, entry.name);
    if (entry.isDirectory()) {
      const found = findFile(candidate, name);
      if (found) {
        return found;
      }
    } else if (entry.name === name) {
      return candidate;
    }
  }
  return undefined;
}

/** Poll until `predicate` holds, so a test can wait on asynchronous extension work. */
export async function waitFor(
  predicate: () => boolean | Promise<boolean>,
  options: { timeout?: number; interval?: number; message?: string } = {},
): Promise<void> {
  const timeout = options.timeout ?? 60_000;
  const interval = options.interval ?? 250;
  const deadline = Date.now() + timeout;
  for (;;) {
    if (await predicate()) {
      return;
    }
    if (Date.now() > deadline) {
      throw new Error(options.message ?? `condition not met within ${timeout}ms`);
    }
    await new Promise((resolve) => setTimeout(resolve, interval));
  }
}
