import * as vscode from "vscode";
import path from "path";
import fs from "fs";
import { askConfigurationBase } from "../common/askers";
import { type XcodeBuildSettings, getSchemes, getTargets } from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { showQuickPick } from "../common/quick-pick";
import type { Destination } from "../destination/types";
import { getCurrentXcodeWorkspacePath } from '../build/utils';
import { parseXml, XmlElement } from '@rgrove/parse-xml';
import type { TestPlan } from './testPlanTypes';
import { ExtensionError } from '../common/errors';

/**
 * Ask user to select target to build
 */
export async function askTestingTarget(
  context: ExtensionContext,
  options: {
    title: string;
    xcworkspace: string;
    force?: boolean;
  },
): Promise<string | null> {
  // Testing target can be cached
  const cachedTarget = context.testingManager.getDefaultTestingTarget();
  if (cachedTarget && !options.force) {
    return cachedTarget;
  }

  // Get from commmand line or from xcode files
  const targets = await getTargets({
    xcworkspace: options.xcworkspace,
  });

  // Target is required for testing
  if (!targets.length) {
    return null;
  }

  // Auto select target if only one found
  if (targets.length === 1 && !options.force) {
    const targetName = targets[0];
    context.testingManager.setDefaultTestingTarget(targetName);
    return targetName;
  }

  // Offer user to select target if multiple found
  const target = await showQuickPick({
    title: options.title,
    items: targets.map((target) => {
      console.log(target, cachedTarget, target === cachedTarget);
      return {
        label: target,
        description: target === cachedTarget ? "(current)" : undefined,
        context: {
          target: target,
        },
      };
    }),
  });

  const targetName = target.context.target;
  context.testingManager.setDefaultTestingTarget(targetName);
  return targetName;
}

/**
 * Ask user to select configuration
 */
export async function askConfigurationForTesting(
  context: ExtensionContext,
  options: {
    xcworkspace: string;
  },
): Promise<string> {
  return await context.withCache("testing.xcodeConfiguration", async () => {
    return await askConfigurationBase({
      xcworkspace: options.xcworkspace,
    });
  });
}

/**
 * Ask user to select simulator or device to run on
 */
export async function askDestinationToTestOn(
  context: ExtensionContext,
  buildSettings: XcodeBuildSettings | null,
): Promise<Destination> {
  // We can remove platforms that are not supported by the project
  const supportedPlatforms = buildSettings?.supportedPlatforms;

  const destinations = await context.destinationsManager.getDestinations({
    platformFilter: supportedPlatforms,
    mostUsedSort: true,
  });

  // If we have cached desination, use it
  const cachedDestination = context.destinationsManager.getSelectedXcodeDestinationForTesting();
  if (cachedDestination) {
    const destination = destinations.find(
      (destination) => destination.id === cachedDestination.id && destination.type === cachedDestination.type,
    );
    if (destination) {
      return destination;
    }
  }

  return selectDestinationForTesting(context, {
    destinations: destinations,
  });
}

export async function selectDestinationForTesting(
  context: ExtensionContext,
  options?: {
    destinations?: Destination[];
  },
): Promise<Destination> {
  const destinations = options?.destinations?.length
    ? options.destinations
    : await context.destinationsManager.getDestinations({
        mostUsedSort: true,
      });

  const selected = await showQuickPick<Destination>({
    title: "Select destination to test on",
    items: [
      ...destinations.map((destination) => {
        return {
          label: destination.name,
          iconPath: new vscode.ThemeIcon(destination.icon),
          detail: destination.quickPickDetails,
          context: destination,
        };
      }),
    ],
  });

  const destination = selected.context;

  context.destinationsManager.setWorkspaceDestinationForTesting(destination);
  return destination;
}

/**
 * Ask user to select scheme to build
 */
export async function askSchemeForTesting(
  context: ExtensionContext,
  options: {
    title?: string;
    xcworkspace: string;
    ignoreCache?: boolean;
  },
): Promise<string> {
  const cachedScheme = context.buildManager.getDefaultSchemeForTesting();
  if (cachedScheme && !options.ignoreCache) {
    return cachedScheme;
  }

  const schemes = await getSchemes({
    xcworkspace: options.xcworkspace,
  });

  const scheme = await showQuickPick({
    title: options?.title ?? "Select scheme to test on",
    items: schemes.map((scheme) => {
      return {
        label: scheme.name,
        context: {
          scheme: scheme,
        },
      };
    }),
  });

  const schemeName = scheme.context.scheme.name;
  context.buildManager.setDefaultSchemeForTesting(schemeName);
  return schemeName;
}

export function parseDefaultTestPlanFile(context: ExtensionContext, rootPath: string): TestPlan {
  const scheme = context.buildManager.getDefaultSchemeForTesting();
  const xcworkspacePath = getCurrentXcodeWorkspacePath(context)
  if (scheme && xcworkspacePath) {
    const schemePath = path.join(xcworkspacePath, "../xcshareddata/xcschemes", scheme + ".xcscheme");
    const content = fs.readFileSync(schemePath, "utf-8")
    const parsed = parseXml(content);
    const testAction = parsed.root?.
      children.find((node) => node instanceof XmlElement && node.name === "TestAction") as XmlElement;
    const testPlanElement = testAction?.children.find((node) => node instanceof XmlElement && node.name === "TestPlans") as XmlElement;
    const testPlanReference = testPlanElement?.children.find((node) => node instanceof XmlElement && node.name === "TestPlanReference") as XmlElement;
    const testPlanReferenceContainer = testPlanReference?.attributes['reference'];
    const [, testPlanPath] = testPlanReferenceContainer.split("container:");

    return JSON.parse(fs.readFileSync(path.join(rootPath, testPlanPath), "utf-8")) as TestPlan;
  }

  throw new ExtensionError("no scheme or workspace found");
}

/**
 * Extracts a code block from the given text starting from the given class name.
 *
 * TODO: use a proper Swift parser to find code blocks
 */
export function extractCodeBlock(className: string, content: string): string | null {
  const lines = content.split('\n');

  let codeBlock = [];
  let stack = 0;
  let inBlock = false;
  let foundEntry = false

  for (const line of lines) {
    foundEntry = foundEntry || line.includes(className)

    if (!foundEntry) {
      continue
    }

    if (line.includes('{')) {
      if (!inBlock) {
        inBlock = true; // Start of the outermost block
      }
      stack++; // Increase stack count for each new block start
    }

    if (inBlock) {
      codeBlock.push(line); // Add the line to the code block
    }

    if (line.includes('}') && inBlock) {
      stack--; // Decrease stack count for each block end
      if (stack === 0) {
        break; // Exit loop after the entire block is captured
      }
    }
  }

  return codeBlock.length > 0 ? codeBlock.join('\n') : null;
}