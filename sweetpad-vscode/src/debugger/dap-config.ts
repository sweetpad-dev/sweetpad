import path from "node:path";

import * as vscode from "vscode";

import type { BuildManager } from "../build/manager";
import {
  askConfiguration,
  askDestinationToRunOn,
  askSchemeForBuild,
  askXcodeWorkspacePath,
  detectWorkspaceType,
  getWorkspaceRoot,
  getXcodeBuildDestinationString,
  prepareDerivedDataPath,
} from "../build/utils";
import { getWorkspaceConfig } from "../common/config";
import { commonLogger } from "../common/logger";
import type { WorkspaceContextService } from "../common/workspace-context";
import type { WorkspaceStateService } from "../common/workspace-state";
import type { DestinationsManager } from "../destination/manager";
import { type RunnableDestination, assertRunnableDestination, findDestinationForTaskInput } from "../destination/utils";
import type { ProgressStatusBar } from "../system/status-bar";

/** The label VS Code gives the extension's own "debugging-launch" task. */
export const DEBUGGING_LAUNCH_TASK_LABEL = "sweetpad: debugging-launch";

/**
 * The fields of a "debugging-launch" task definition that pick what to launch. A task written
 * in tasks.json can carry any of them.
 */
export type DebuggingLaunchTaskDefinition = {
  scheme?: string;
  configuration?: string;
  destinationId?: string;
  destination?: string;
  simulator?: string;
  launchArgs?: string[];
  launchEnv?: { [key: string]: string };
};

/**
 * The "debugging-launch" task `preLaunchTask` names, or undefined when it names another task
 * or none.
 *
 * The task is looked up among the "sweetpad" tasks VS Code knows, so a tasks.json copy under
 * its own label is recognized and its scheme, destination and launch arguments are kept. The
 * extension's own label is recognized even when that lookup fails.
 */
export async function findDebuggingLaunchTask(
  preLaunchTask: unknown,
): Promise<DebuggingLaunchTaskDefinition | undefined> {
  if (typeof preLaunchTask !== "string" || preLaunchTask.length === 0) {
    return undefined;
  }

  let tasks: vscode.Task[] = [];
  try {
    tasks = await vscode.tasks.fetchTasks({ type: "sweetpad" });
  } catch (error) {
    commonLogger.warn("Could not list SweetPad tasks", { error });
  }

  const matches = tasks.filter(
    (task) =>
      task.definition.action === "debugging-launch" &&
      (task.name === preLaunchTask || `${task.source}: ${task.name}` === preLaunchTask),
  );
  // The extension's own task and a tasks.json copy can share a label; the copy is the one with
  // the user's fields on it.
  const task = matches.find((match) => namesLaunchTarget(match.definition)) ?? matches[0];
  if (task) {
    return task.definition as DebuggingLaunchTaskDefinition;
  }
  return preLaunchTask === DEBUGGING_LAUNCH_TASK_LABEL ? {} : undefined;
}

function namesLaunchTarget(definition: vscode.TaskDefinition): boolean {
  const keys = ["scheme", "configuration", "destinationId", "destination", "simulator", "launchArgs", "launchEnv"];
  return keys.some((key) => definition[key] !== undefined);
}

export type DapSelectionDeps = {
  workspaceState: WorkspaceStateService;
  workspaceContext: WorkspaceContextService;
  buildManager: BuildManager;
  destinationsManager: DestinationsManager;
  progressStatusBar: ProgressStatusBar;
};

/** What `sweetpad dap` builds and where it runs it, settled before the session starts. */
export type DapSelection = {
  xcworkspace: string;
  workspaceRoot: string;
  scheme: string;
  configuration: string;
  /** The `destination` field for the CLI. */
  destination: string;
  /** The destination it names, when it is one SweetPad knows. */
  target: RunnableDestination | undefined;
};

/**
 * Whether the session needs a scheme and destination from SweetPad. A config naming `program`
 * is plain lldb-dap, and an attach by `pid` alone names a process on this Mac.
 */
export function needsDapSelection(
  config: vscode.DebugConfiguration,
  task: DebuggingLaunchTaskDefinition | undefined,
): boolean {
  if (typeof config.program === "string") {
    return false;
  }
  const attachesByPid =
    task === undefined && config.request === "attach" && config.pid !== undefined && config.pid !== null;
  return !attachesByPid;
}

/**
 * Settle the project, scheme, configuration and destination for `sweetpad dap`.
 *
 * A value in the launch configuration wins, then one on the "debugging-launch" task, then the
 * selection SweetPad keeps. Anything still missing is asked for through the same pickers the
 * "debugging-launch" task uses, which also remember the answer.
 */
export async function resolveDapSelection(
  deps: DapSelectionDeps,
  config: vscode.DebugConfiguration,
  task: DebuggingLaunchTaskDefinition | undefined,
): Promise<DapSelection> {
  deps.progressStatusBar.updateText("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath({
    workspaceState: deps.workspaceState,
    workspaceContext: deps.workspaceContext,
    buildManager: deps.buildManager,
  });
  const workspaceRoot = getWorkspaceRoot({ xcworkspace: xcworkspace, workspaceContext: deps.workspaceContext });

  const scheme =
    stringField(config, "scheme") ??
    task?.scheme ??
    (await askSchemeForBuild(deps.progressStatusBar, deps.buildManager, {
      title: "Select scheme to debug",
      xcworkspace: xcworkspace,
    }));

  const configuration =
    stringField(config, "configuration") ??
    task?.configuration ??
    (await askConfiguration(deps.progressStatusBar, deps.buildManager, { xcworkspace: xcworkspace }));

  const destination = await resolveDapDestination(deps, {
    config: config,
    task: task,
    workspaceRoot: workspaceRoot,
    scheme: scheme,
    configuration: configuration,
    xcworkspace: xcworkspace,
  });

  return {
    xcworkspace: xcworkspace,
    workspaceRoot: workspaceRoot,
    scheme: scheme,
    configuration: configuration,
    destination: destination.value,
    target: destination.target,
  };
}

async function resolveDapDestination(
  deps: DapSelectionDeps,
  options: {
    config: vscode.DebugConfiguration;
    task: DebuggingLaunchTaskDefinition | undefined;
    workspaceRoot: string;
    scheme: string;
    configuration: string;
    xcworkspace: string;
  },
): Promise<{ value: string; target: RunnableDestination | undefined }> {
  const fromConfig = stringField(options.config, "destination");
  if (fromConfig !== undefined) {
    return { value: fromConfig, target: undefined };
  }

  const task = options.task;
  const fromTask = task?.destination ?? task?.destinationId ?? task?.simulator;
  if (task !== undefined && fromTask !== undefined) {
    const destinations = await deps.destinationsManager.getDestinations();
    const found = findDestinationForTaskInput(destinations, task);
    if (found) {
      assertRunnableDestination(found, "launch");
    }
    // A raw `-destination` string goes to the CLI as written, so a variant such as Mac
    // Catalyst survives. The CLI reads a UDID the same way it reads its own `--on`.
    const value = task.destination ?? (found ? dapDestination(found) : fromTask);
    return { value: value, target: found };
  }

  const destination = await askDestinationToRunOn(deps.progressStatusBar, deps.destinationsManager, {
    workspaceRoot: options.workspaceRoot,
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: undefined,
    xcworkspace: options.xcworkspace,
    action: "launch",
  });
  assertRunnableDestination(destination, "launch");
  return { value: dapDestination(destination), target: destination };
}

/**
 * The CLI's `destination` for a destination SweetPad picked: `mac` for this Mac, and a UDID for
 * a simulator or device. A simulator built for x86_64 under Rosetta is named by its full
 * `-destination` string instead, since a UDID alone would drop the architecture.
 */
export function dapDestination(destination: RunnableDestination): string {
  switch (destination.type) {
    case "macOS":
      return "mac";
    case "iOSSimulator":
    case "watchOSSimulator":
    case "tvOSSimulator":
    case "visionOSSimulator":
      return getWorkspaceConfig("build.rosettaDestination")
        ? getXcodeBuildDestinationString({ destination: destination })
        : destination.udid;
    case "iOSDevice":
    case "watchOSDevice":
    case "tvOSDevice":
    case "visionOSDevice":
      return destination.udid;
  }
}

/**
 * Turn a `sweetpad-lldb` configuration into the launch or attach request `sweetpad dap` reads.
 *
 * The "debugging-launch" pre-launch task is dropped, because the adapter builds and launches
 * the app itself, and the attach that followed it becomes a launch. An attach with no such
 * task stays an attach to a running app. Fields the configuration already has are kept;
 * `selection` fills the rest. `codelldbAttributes` belongs to the CodeLLDB route and is
 * dropped, while an `lldb` object passes through to lldb-dap as written.
 */
export function buildDapConfig(
  config: vscode.DebugConfiguration,
  options: {
    task: DebuggingLaunchTaskDefinition | undefined;
    selection: DapSelection | undefined;
    folder: vscode.WorkspaceFolder | undefined;
  },
): vscode.DebugConfiguration {
  const { task, selection } = options;
  const result: vscode.DebugConfiguration = { ...config };
  delete result.codelldbAttributes;

  if (task !== undefined) {
    delete result.preLaunchTask;
    if (result.request === "attach") {
      result.request = "launch";
    }
  }

  if (selection === undefined) {
    if (stringField(config, "cwd") === undefined && options.folder) {
      result.cwd = options.folder.uri.fsPath;
    }
    return result;
  }

  const namesContainer = ["cwd", "workspace", "project"].some((key) => stringField(config, key) !== undefined);
  if (!namesContainer) {
    Object.assign(result, dapContainerFields(selection.xcworkspace));
  }
  result.cwd = stringField(config, "cwd") ?? dapWorkingDirectory(selection);
  result.scheme = selection.scheme;
  result.configuration = selection.configuration;
  result.destination = selection.destination;

  if (config.args === undefined) {
    const args = task?.launchArgs ?? getWorkspaceConfig("build.launchArgs") ?? [];
    if (args.length > 0) {
      result.args = args;
    }
  }
  if (config.env === undefined) {
    const env = task?.launchEnv ?? getWorkspaceConfig("build.launchEnv") ?? {};
    if (Object.keys(env).length > 0) {
      result.env = env;
    }
  }
  if (config.xcodebuildArgs === undefined) {
    const xcodebuildArgs = dapXcodebuildArgs({ workspaceRoot: selection.workspaceRoot });
    if (xcodebuildArgs.length > 0) {
      result.xcodebuildArgs = xcodebuildArgs;
    }
  }
  return result;
}

/**
 * The container fields for the project SweetPad has selected. A project's embedded
 * `project.xcworkspace` is named by its project, which xcodebuild treats the same. A Swift
 * package has no field: the CLI finds it from `cwd`.
 */
function dapContainerFields(xcworkspace: string): { workspace?: string; project?: string } {
  if (detectWorkspaceType(xcworkspace) === "spm") {
    return {};
  }
  const embedded = `.xcodeproj${path.sep}project.xcworkspace`;
  if (xcworkspace.endsWith(embedded)) {
    return { project: path.dirname(xcworkspace) };
  }
  if (xcworkspace.endsWith(".xcodeproj")) {
    return { project: xcworkspace };
  }
  return { workspace: xcworkspace };
}

/** The directory the CLI resolves from: a Swift package's own directory, else the workspace folder. */
function dapWorkingDirectory(selection: DapSelection): string {
  if (detectWorkspaceType(selection.xcworkspace) === "spm") {
    return path.dirname(selection.xcworkspace);
  }
  return selection.workspaceRoot;
}

/**
 * The extra xcodebuild arguments the extension's own builds pass: `sweetpad.build.args`, the
 * DerivedData location the extension and its index share, and `-allowProvisioningUpdates` when
 * `sweetpad.build.allowProvisioningUpdates` is on. The DerivedData path is made absolute here,
 * because the CLI resolves a relative one against the project's directory and the extension
 * resolves it against the workspace folder.
 */
export function dapXcodebuildArgs(options: { workspaceRoot: string }): string[] {
  const buildArgs: string[] = getWorkspaceConfig("build.args") ?? [];
  const args: string[] = [];
  for (let i = 0; i < buildArgs.length; i++) {
    if (buildArgs[i] === "-derivedDataPath") {
      i++; // and its value
      continue;
    }
    args.push(buildArgs[i]);
  }

  const derivedDataPath = prepareDerivedDataPath({ workspaceRoot: options.workspaceRoot });
  if (derivedDataPath) {
    args.push("-derivedDataPath", derivedDataPath);
  }
  const allowProvisioningUpdates = getWorkspaceConfig("build.allowProvisioningUpdates") ?? true;
  if (allowProvisioningUpdates && !args.includes("-allowProvisioningUpdates")) {
    args.push("-allowProvisioningUpdates");
  }
  return args;
}

/** A string field of the configuration. An empty string counts as unset, as it does for the CLI. */
function stringField(config: vscode.DebugConfiguration, key: string): string | undefined {
  const value = config[key];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}
