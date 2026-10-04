import * as vscode from "vscode";

import { getWorkspaceConfig } from "../common/config";
import { type ErrorMessageAction, ExtensionError } from "../common/errors";
import type { Destination } from "../destination/types";
import type { SweetpadCliStatus } from "./cli";

/** `sweetpad.debugger.adapter`: which debugger runs a `sweetpad-lldb` session. */
export type DebuggerAdapter = "auto" | "sweetpad" | "codelldb";

export const CODELLDB_EXTENSION_ID = "vadimcn.vscode-lldb";

const SWEETPAD_CLI_TOOL_ID = "sweetpad-cli";
const INSTALL_CLI_COMMAND = "brew install sweetpad-dev/tap/sweetpad";
const UPGRADE_CLI_COMMAND = "brew upgrade sweetpad-dev/tap/sweetpad";

export function getDebuggerAdapterSetting(): DebuggerAdapter {
  const value = getWorkspaceConfig("debugger.adapter");
  return value === "sweetpad" || value === "codelldb" ? value : "auto";
}

export function isCodelldbInstalled(): boolean {
  return vscode.extensions.getExtension(CODELLDB_EXTENSION_ID) !== undefined;
}

/** A physical device the CLI can't debug, because it has no devicectl (iOS 16 and older). */
export type DeviceNeedingCodelldb = { name: string; osVersion: string };

/**
 * The device `destination` is, when the CLI can't debug on it. The CLI reaches devices only
 * through devicectl, so this is the same test the extension's own run uses to pick ios-deploy.
 */
export function deviceNeedingCodelldb(destination: Destination): DeviceNeedingCodelldb | undefined {
  switch (destination.type) {
    case "iOSDevice":
    case "watchOSDevice":
    case "tvOSDevice":
    case "visionOSDevice":
      return destination.supportsDevicectl ? undefined : { name: destination.name, osVersion: destination.osVersion };
    default:
      return undefined;
  }
}

export type UnavailableRouteReason =
  /** "auto": neither a CLI with `dap` nor CodeLLDB. */
  | { why: "nothing-installed"; cli: SweetpadCliStatus }
  /** "auto": the destination is a device only CodeLLDB can debug, and CodeLLDB is missing. */
  | { why: "device-needs-codelldb"; device: DeviceNeedingCodelldb }
  /** "sweetpad": the CLI is missing or has no `dap`. */
  | { why: "cli-unusable"; cli: SweetpadCliStatus }
  /** "codelldb": CodeLLDB is missing. */
  | { why: "codelldb-missing" };

export type DebugRoute =
  | { kind: "sweetpad"; cliPath: string }
  | { kind: "codelldb" }
  | { kind: "unavailable"; reason: UnavailableRouteReason };

/**
 * Pick the debugger for one session.
 *
 * `cli` is undefined when it was not looked up ("codelldb" never needs it), and
 * `deviceNeedingCodelldb` is set only when the session targets a device the CLI can't debug.
 */
export function chooseDebugRoute(options: {
  adapter: DebuggerAdapter;
  cli: SweetpadCliStatus | undefined;
  codelldbInstalled: boolean;
  deviceNeedingCodelldb: DeviceNeedingCodelldb | undefined;
}): DebugRoute {
  const cli: SweetpadCliStatus = options.cli ?? { kind: "missing", configuredPath: undefined };

  switch (options.adapter) {
    case "codelldb":
      return options.codelldbInstalled
        ? { kind: "codelldb" }
        : { kind: "unavailable", reason: { why: "codelldb-missing" } };
    case "sweetpad":
      return cli.kind === "ready"
        ? { kind: "sweetpad", cliPath: cli.path }
        : { kind: "unavailable", reason: { why: "cli-unusable", cli: cli } };
    case "auto":
      if (cli.kind === "ready") {
        if (options.deviceNeedingCodelldb === undefined) {
          return { kind: "sweetpad", cliPath: cli.path };
        }
        return options.codelldbInstalled
          ? { kind: "codelldb" }
          : { kind: "unavailable", reason: { why: "device-needs-codelldb", device: options.deviceNeedingCodelldb } };
      }
      return options.codelldbInstalled
        ? { kind: "codelldb" }
        : { kind: "unavailable", reason: { why: "nothing-installed", cli: cli } };
  }
}

/** The error a session fails with when no debugger can run it, with buttons that fix it. */
export function unavailableRouteError(reason: UnavailableRouteReason): ExtensionError {
  switch (reason.why) {
    case "nothing-installed": {
      const { problem, fix } = cliProblem(reason.cli);
      return new ExtensionError(
        `Debugging needs the SweetPad CLI or the CodeLLDB extension. ${problem} ${fix}, or install CodeLLDB.`,
        { actions: [cliAction(reason.cli), installCodelldbAction()] },
      );
    }
    case "device-needs-codelldb":
      return new ExtensionError(
        `Debugging on ${reason.device.name} (${reason.device.osVersion}) needs the CodeLLDB extension. The SweetPad CLI debugs devices on iOS 17 and later.`,
        { actions: [installCodelldbAction()] },
      );
    case "cli-unusable": {
      const { problem, fix } = cliProblem(reason.cli);
      return new ExtensionError(`The "sweetpad.debugger.adapter" setting is "sweetpad". ${problem} ${fix}.`, {
        actions: [cliAction(reason.cli), openSettingsAction()],
      });
    }
    case "codelldb-missing":
      return new ExtensionError(
        `The "sweetpad.debugger.adapter" setting is "codelldb", but the CodeLLDB extension is not installed.`,
        { actions: [installCodelldbAction(), openSettingsAction()] },
      );
  }
}

/** What is wrong with the CLI, and the start of a clause that fixes it. */
function cliProblem(cli: SweetpadCliStatus): { problem: string; fix: string } {
  switch (cli.kind) {
    case "missing":
      return cli.configuredPath !== undefined
        ? {
            problem: `No SweetPad CLI was found at ${cli.configuredPath}, the path in "sweetpad.debugger.cliPath".`,
            fix: "Fix the path",
          }
        : { problem: "The SweetPad CLI was not found.", fix: `Install it with '${INSTALL_CLI_COMMAND}'` };
    case "no-dap":
      return {
        problem: `The SweetPad CLI at ${cli.path} is too old to debug.`,
        fix: `Update it with '${UPGRADE_CLI_COMMAND}'`,
      };
    case "ready":
      return { problem: "", fix: "" };
  }
}

/**
 * Install or update the CLI through the Tools view's installer. `brew install` also upgrades a
 * formula that is installed but outdated.
 */
function cliAction(cli: SweetpadCliStatus): ErrorMessageAction {
  return {
    label: cli.kind === "no-dap" ? "Update SweetPad CLI" : "Install SweetPad CLI",
    callback: () => void vscode.commands.executeCommand("sweetpad.tools.install", SWEETPAD_CLI_TOOL_ID),
  };
}

function installCodelldbAction(): ErrorMessageAction {
  return {
    label: "Install CodeLLDB",
    callback: () => void vscode.commands.executeCommand("workbench.extensions.installExtension", CODELLDB_EXTENSION_ID),
  };
}

function openSettingsAction(): ErrorMessageAction {
  return {
    label: "Open settings",
    callback: () => void vscode.commands.executeCommand("workbench.action.openSettings", "sweetpad.debugger"),
  };
}
