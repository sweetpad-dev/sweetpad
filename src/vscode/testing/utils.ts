import * as vscode from "vscode";

import type { UserAsker } from "../../core/asker/types";
import { askConfigurationBase } from "../../core/askers";
import type { BuildManager } from "../../core/build/manager";
import { type XcodeBuildSettings, type XcodeCliDeps, getSchemes, getTargets } from "../../core/cli/scripts";
import type { ConfigProvider } from "../../core/config/types";
import type { DestinationPlatform } from "../../core/destination/constants";
import type { DestinationsManager } from "../../core/destination/manager";
import type { Destination } from "../../core/destination/types";
import { splitSupportedDestinatinos } from "../../core/destination/utils";
import { type QuickPickItem, showQuickPick } from "../quick-pick";
import type { ProgressStatusBar } from "../system/status-bar";
import type { TestingManager } from "./manager";

/**
 * Ask user to select target to build
 */
export async function askTestingTarget(options: {
  testingManager: TestingManager;
  cliDeps: XcodeCliDeps;
  title: string;
  xcworkspace: string;
  force?: boolean;
}): Promise<string | null> {
  // Testing target can be cached
  const cachedTarget = options.testingManager.getDefaultTestingTarget();
  if (cachedTarget && !options.force) {
    return cachedTarget;
  }

  // Get from commmand line or from xcode files
  const targets = await getTargets(options.cliDeps, {
    xcworkspace: options.xcworkspace,
  });

  // Target is required for testing
  if (!targets.length) {
    return null;
  }

  // Auto select target if only one found
  if (targets.length === 1 && !options.force) {
    const targetName = targets[0];
    options.testingManager.setDefaultTestingTarget(targetName);
    return targetName;
  }

  // Offer user to select target if multiple found
  const target = await showQuickPick({
    title: options.title,
    items: targets.map((t) => {
      return {
        label: t,
        description: t === cachedTarget ? "(current)" : undefined,
        context: {
          target: t,
        },
      };
    }),
  });

  const targetName = target.context.target;
  options.testingManager.setDefaultTestingTarget(targetName);
  return targetName;
}

/**
 * Ask user to select configuration
 */
export async function askConfigurationForTesting(options: {
  asker: UserAsker;
  buildManager: BuildManager;
  config: ConfigProvider;
  cliDeps: XcodeCliDeps;
  xcworkspace: string;
}): Promise<string> {
  const fromConfig = options.config.get("testing.configuration");
  if (fromConfig) {
    return fromConfig;
  }
  const cached = options.buildManager.getDefaultConfigurationForTesting();
  if (cached) {
    return cached;
  }
  const selected = await askConfigurationBase(
    { ...options.cliDeps, asker: options.asker },
    {
      xcworkspace: options.xcworkspace,
    },
  );
  options.buildManager.setDefaultConfigurationForTesting(selected);
  return selected;
}

/**
 * Ask user to select simulator or device to run on
 */
export async function askDestinationToTestOn(
  destinationsManager: DestinationsManager,
  buildSettings: XcodeBuildSettings | null,
): Promise<Destination> {
  // We can remove platforms that are not supported by the project
  const supportedPlatforms = buildSettings?.supportedPlatforms;

  const destinations = await destinationsManager.getDestinations({
    mostUsedSort: true,
  });

  // If we have cached desination, use it
  const cachedDestination = destinationsManager.getSelectedXcodeDestinationForTesting();
  if (cachedDestination) {
    const destination = destinations.find((d) => d.id === cachedDestination.id && d.type === cachedDestination.type);
    if (destination) {
      return destination;
    }
  }

  return await selectDestinationForTesting(destinationsManager, {
    destinations: destinations,
    supportedPlatforms: supportedPlatforms,
  });
}

export async function selectDestinationForTesting(
  destinationsManager: DestinationsManager,
  options: {
    destinations: Destination[];
    supportedPlatforms: DestinationPlatform[] | undefined;
  },
): Promise<Destination> {
  const { supported, unsupported } = splitSupportedDestinatinos({
    destinations: options.destinations,
    supportedPlatforms: options.supportedPlatforms,
  });

  const supportedItems: QuickPickItem<Destination>[] = supported.map((destination) => ({
    label: destination.name,
    iconPath: new vscode.ThemeIcon(destination.icon),
    detail: destination.quickPickDetails,
    context: destination,
  }));
  const unsupportedItems: QuickPickItem<Destination>[] = unsupported.map((destination) => ({
    label: destination.name,
    iconPath: new vscode.ThemeIcon(destination.icon),
    detail: destination.quickPickDetails,
    context: destination,
  }));

  const items: QuickPickItem<Destination>[] = [];
  if (unsupported.length === 0 && supported.length === 0) {
    // Show that no destinations found
    items.push({
      label: "No destinations found",
      kind: vscode.QuickPickItemKind.Separator,
      context: supported[0],
    });
  } else if (supported.length > 0 && unsupported.length > 0) {
    // Split supported and unsupported destinations
    items.push({
      label: "Supported",
      kind: vscode.QuickPickItemKind.Separator,
      context: supported[0],
    });
    items.push(...supportedItems);
    items.push({
      label: "Other",
      kind: vscode.QuickPickItemKind.Separator,
      context: supported[0],
    });
    items.push(...unsupportedItems);
  } else {
    // Just make flat list, one is empty and another is not
    items.push(...supportedItems);
    items.push(...unsupportedItems);
  }

  const selected = await showQuickPick<Destination>({
    title: "Select destination to test on",
    items: items,
  });

  const destination = selected.context;

  destinationsManager.setWorkspaceDestinationForTesting(destination);
  return destination;
}

/**
 * Ask user to select scheme to build
 */
export async function askSchemeForTesting(options: {
  progress: ProgressStatusBar;
  buildManager: BuildManager;
  cliDeps: XcodeCliDeps;
  title?: string;
  xcworkspace: string;
  ignoreCache?: boolean;
}): Promise<string> {
  const cachedScheme = options.buildManager.getDefaultSchemeForTesting();
  if (cachedScheme && !options.ignoreCache) {
    return cachedScheme;
  }

  options.progress.updateText("Searching for scheme");
  const schemes = await getSchemes(options.cliDeps, {
    xcworkspace: options.xcworkspace,
  });

  const scheme = await showQuickPick({
    title: options.title ?? "Select scheme to test on",
    items: schemes.map((s) => {
      return {
        label: s.name,
        context: {
          scheme: s,
        },
      };
    }),
  });

  const schemeName = scheme.context.scheme.name;
  options.buildManager.setDefaultSchemeForTesting(schemeName);
  return schemeName;
}
