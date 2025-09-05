import path from "node:path";
import * as vscode from "vscode";
import { type QuickPickItem, showQuickPick } from "../common/quick-pick";

import { askConfigurationBase } from "../common/askers";
import { type XcodeBuildSettings, getSchemes } from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { ExtensionError } from "../common/errors";
import { createDirectory, findFilesRecursive, isFileExists, removeDirectory } from "../common/files";
import { commonLogger } from "../common/logger";
import type { DestinationPlatform } from "../destination/constants";
import type { Destination } from "../destination/types";
import { splitSupportedDestinatinos } from "../destination/utils";
import type { SimulatorDestination } from "../simulators/types";

export type SelectedDestination = {
  type: "simulator" | "device";
  udid: string;
  name?: string;
};

/**
 * Ask user to select one of the Booted/Shutdown simulators
 */
export async function askSimulator(
  context: ExtensionContext,
  options: {
    title: string;
    state: "Booted" | "Shutdown";
    error: string;
  },
): Promise<SimulatorDestination> {
  let simulators = await context.destinationsManager.getSimulators({
    sort: true,
  });

  if (options?.state) {
    simulators = simulators.filter((simulator) => simulator.state === options.state);
  }

  if (simulators.length === 0) {
    throw new ExtensionError(options.error);
  }
  if (simulators.length === 1) {
    return simulators[0];
  }

  const selected = await showQuickPick({
    title: options.title,
    items: simulators.map((simulator) => {
      return {
        label: simulator.label,
        context: {
          simulator: simulator,
        },
      };
    }),
  });

  return selected.context.simulator;
}

/**
 * Ask user to select simulator or device to run on
 */
export async function askDestinationToRunOn(
  context: ExtensionContext,
  buildSettings: XcodeBuildSettings | null,
): Promise<Destination> {
  // We can remove platforms that are not supported by the project
  const supportedPlatforms = buildSettings?.supportedPlatforms;

  context.updateProgressStatus("Searching for destinations");
  const destinations = await context.destinationsManager.getDestinations({
    mostUsedSort: true,
  });

  // If we have cached desination, use it
  const cachedDestination = context.destinationsManager.getSelectedXcodeDestinationForBuild();
  if (cachedDestination) {
    const destination = destinations.find(
      (destination) => destination.id === cachedDestination.id && destination.type === cachedDestination.type,
    );
    if (destination) {
      return destination;
    }
  }

  return await selectDestinationForBuild(context, {
    destinations: destinations,
    supportedPlatforms: supportedPlatforms,
  });
}

export async function selectDestinationForBuild(
  context: ExtensionContext,
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
    });
  } else if (supported.length > 0 && unsupported.length > 0) {
    // Split supported and unsupported destinations
    items.push({
      label: "Supported platforms",
      kind: vscode.QuickPickItemKind.Separator,
    });
    items.push(...supportedItems);
    items.push({
      label: "Other",
      kind: vscode.QuickPickItemKind.Separator,
    });
    items.push(...unsupportedItems);
  } else {
    // Just make flat list, one is empty and another is not
    items.push(...supportedItems);
    items.push(...unsupportedItems);
  }

  const selected = await showQuickPick<Destination>({
    title: "Select destination to run on",
    items: items,
  });

  const destination = selected.context;

  context.destinationsManager.setWorkspaceDestinationForBuild(destination);
  return destination;
}

export async function getDestinationById(
  context: ExtensionContext,
  options: { destinationId: string },
): Promise<Destination> {
  const desinations = await context.destinationsManager.getDestinations();
  const destination = desinations.find((destination) => destination.id === options.destinationId);

  if (destination) {
    return destination;
  }

  throw new ExtensionError("Destination not found", {
    context: {
      destinationId: options.destinationId,
    },
  });
}

/**
 * Ask user to select scheme to build
 */
export async function askSchemeForBuild(
  context: ExtensionContext,
  options: {
    title?: string;
    xcworkspace: string;
    ignoreCache?: boolean;
  },
): Promise<string> {
  context.updateProgressStatus("Searching for scheme");

  const cachedScheme = context.buildManager.getDefaultSchemeForBuild();
  if (cachedScheme && !options.ignoreCache) {
    return cachedScheme;
  }

  const schemes = await getSchemes({
    xcworkspace: options.xcworkspace,
  });

  const scheme = await showQuickPick({
    title: options?.title ?? "Select scheme to build",
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
  context.buildManager.setDefaultSchemeForBuild(schemeName);
  return schemeName;
}

/**
 * Get the path of the current workspace
 * @throws {ExtensionError} If no workspace is open
 */
export function getWorkspacePath(): string {
  try {
    if (!vscode.workspace.workspaceFolders || vscode.workspace.workspaceFolders.length === 0) {
      throw new ExtensionError("No workspace folder found. Please open a folder or workspace first.");
    }
    
    const workspaceFolder = vscode.workspace.workspaceFolders[0].uri.fsPath;
    if (!workspaceFolder) {
      throw new ExtensionError("Invalid workspace folder path");
    }
    return workspaceFolder;
  } catch (error) {
    // Log the error for debugging purposes
    commonLogger.error("Failed to get workspace path", { error });
    throw new ExtensionError("No workspace folder found. Please open a folder or workspace first.");
  }
}

/**
 * Prepare storage path for the extension. It's a folder where we store all intermediate files
 */
export async function prepareStoragePath(context: ExtensionContext): Promise<string> {
  const storagePath = context.storageUri?.fsPath;
  if (!storagePath) {
    throw new ExtensionError("No storage path found");
  }
  // Creatre folder at storagePath, because vscode doesn't create it automatically
  await createDirectory(storagePath);
  return storagePath;
}

/**
 * Prepare bundle directory for the given schema in the storage path
 */
export async function prepareBundleDir(context: ExtensionContext, schema: string): Promise<string> {
  const storagePath = await prepareStoragePath(context);

  const bundleDir = path.join(storagePath, "bundle", schema);

  // Remove old bundle if exists
  await removeDirectory(bundleDir);

  // Remove old .xcresult if exists
  const xcresult = path.join(storagePath, "bundle", `${schema}.xcresult`);
  await removeDirectory(xcresult);

  return bundleDir;
}

export function prepareDerivedDataPath(): string | null {
  const configPath = getWorkspaceConfig("build.derivedDataPath");

  // No config -> path will be provided by xcodebuild
  if (!configPath) {
    return null;
  }

  // Expand relative path to absolute
  let derivedDataPath: string = configPath;
  if (!path.isAbsolute(configPath)) {
    // Example: .biuld/ -> /Users/username/Projects/project/.build
    derivedDataPath = path.join(getWorkspacePath(), configPath);
  }

  return derivedDataPath;
}

export function getCurrentXcodeWorkspacePath(context: ExtensionContext): string | undefined {
  const configPath = getWorkspaceConfig("build.xcodeWorkspacePath");
  if (configPath) {
    context.updateWorkspaceState("build.xcodeWorkspacePath", undefined);
    if (path.isAbsolute(configPath)) {
      return configPath;
    }
    return path.join(getWorkspacePath(), configPath);
  }

  const cachedPath = context.getWorkspaceState("build.xcodeWorkspacePath");
  if (cachedPath) {
    return cachedPath;
  }

  return undefined;
}

/**
 * Get the path of the currently selected Xcode workspace or ask user to select one
 */
export async function askXcodeWorkspacePath(context: ExtensionContext, specificPath?: string): Promise<string> {
  context.updateProgressStatus("Searching for workspace");
  
  // If a specific path is provided, use it directly
  if (specificPath) {
    return specificPath;
  }
  
  const xcworkspace = getCurrentXcodeWorkspacePath(context);
  if (xcworkspace) {
    return xcworkspace;
  }

  const selectedPath = await selectXcodeWorkspace({
    autoselect: true,
  });

  context.updateWorkspaceState("build.xcodeWorkspacePath", selectedPath);
  context.buildManager.refresh();
  return selectedPath;
}

export async function askConfiguration(
  context: ExtensionContext,
  options: {
    xcworkspace: string;
  },
): Promise<string> {
  context.updateProgressStatus("Searching for build configuration");

  const fromConfig = getWorkspaceConfig("build.configuration");
  if (fromConfig) {
    return fromConfig;
  }
  const cached = context.buildManager.getDefaultConfigurationForBuild();
  if (cached) {
    return cached;
  }
  const selected = await askConfigurationBase({
    xcworkspace: options.xcworkspace,
  });
  context.buildManager.setDefaultConfigurationForBuild(selected);
  return selected;
}

/**
 * Detect xcode workspace in the given directory
 */
export async function detectXcodeWorkspacesPaths(): Promise<string[]> {
  const workspace = getWorkspacePath();

  // Get all files that end with .xcworkspace (4 depth)
  const xcworkspacePaths = await findFilesRecursive({
    directory: workspace,
    depth: 4,
    maxResults: 20, // Limit workspace files
    matcher: (file) => {
      return file.name.endsWith(".xcworkspace");
    },
  });

  // Also look for Package.swift files for SPM projects
  const packageSwiftPaths = await findFilesRecursive({
    directory: workspace,
    depth: 4,
    maxResults: 30, // Limit Package.swift files
    matcher: (file) => {
      return file.name === "Package.swift";
    },
  });

  // Look for BUILD.bazel files for Bazel projects
  const bazelBuildPaths = await findFilesRecursive({
    directory: workspace,
    depth: 4,
    maxResults: 50, // Limit Bazel files to prevent performance issues
    matcher: (file) => {
      return file.name === "BUILD.bazel" || file.name === "BUILD";
    },
  });

  // Combine all types of paths
  return [...xcworkspacePaths, ...packageSwiftPaths, ...bazelBuildPaths];
}

/**
 * Find xcode workspace in the given directory and ask user to select it
 */
export async function selectXcodeWorkspace(options: { autoselect: boolean }): Promise<string> {
  const workspacePath = getWorkspacePath();

  // Get all files that end with .xcworkspace (4 depth), Package.swift files, and BUILD.bazel files
  const paths = await detectXcodeWorkspacesPaths();

  // No files, nothing to do
  if (paths.length === 0) {
    throw new ExtensionError("No xcode workspaces, SPM packages, or Bazel projects found", {
      context: {
        cwd: workspacePath,
      },
    });
  }

  // One file, use it and save it to the cache
  if (paths.length === 1 && options.autoselect) {
    const path = paths[0];
    let projectType: string;
    if (path.endsWith("Package.swift")) {
      projectType = "SPM package";
    } else if (path.endsWith("BUILD.bazel") || path.endsWith("BUILD")) {
      projectType = "Bazel project";
    } else {
      projectType = "Xcode workspace";
    }
    commonLogger.log(`${projectType} was detected`, {
      workspace: workspacePath,
      path: path,
    });
    return path;
  }

  const podfilePath = path.join(workspacePath, "Podfile");
  const isCocoaProject = await isFileExists(podfilePath);

  // More then one, ask user to select
  const selected = await showQuickPick({
    title: "Select xcode workspace, SPM package, or Bazel project",
    items: paths
      .sort((a, b) => {
        // Sort by depth to show less nested paths first
        const aDepth = a.split(path.sep).length;
        const bDepth = b.split(path.sep).length;
        return aDepth - bDepth;
      })
      .map((xwPath) => {
        // show only relative path, to make it more readable
        const relativePath = path.relative(workspacePath, xwPath);
        const parentDir = path.dirname(relativePath);

        const isInRootDir = parentDir === ".";
        const isCocoaPods = isInRootDir && isCocoaProject;
        const isSPMPackage = xwPath.endsWith("Package.swift");
        const isBazelProject = xwPath.endsWith("BUILD.bazel") || xwPath.endsWith("BUILD");

        let detail: string | undefined;
        if (isSPMPackage) {
          detail = "Swift Package Manager";
        } else if (isBazelProject) {
          detail = "Bazel";
        } else if (isCocoaPods && isInRootDir) {
          detail = "CocoaPods (recommended)";
        } else if (!isInRootDir && parentDir.endsWith(".xcodeproj")) {
          detail = "Xcode";
        }
        // todo: add workspace with multiple projects

        return {
          label: relativePath,
          detail: detail,
          context: {
            path: xwPath,
          },
        };
      }),
  });
  return selected.context.path;
}

export async function restartSwiftLSP() {
  // Restart SourceKit Language Server
  try {
    await vscode.commands.executeCommand("swift.restartLSPServer");
  } catch (error) {
    commonLogger.warn("Error restarting SourceKit Language Server", {
      error: error,
    });
  }
}

/**
 * Find the Bazel workspace root by looking for WORKSPACE files
 */
export function findBazelWorkspaceRoot(startPath: string): string {
  let currentPath = path.dirname(startPath);
  const fs = require('fs');
  
  // Walk up the directory tree looking for Bazel workspace indicators
  while (currentPath !== path.dirname(currentPath)) { // Stop at filesystem root
    // Check for Bazel workspace files
    const workspaceFiles = ['WORKSPACE', 'WORKSPACE.bazel', 'MODULE.bazel'];
    
    for (const workspaceFile of workspaceFiles) {
      const workspaceFilePath = path.join(currentPath, workspaceFile);
      try {
        if (fs.existsSync(workspaceFilePath)) {
          return currentPath;
        }
      } catch (error) {
        // Ignore permission errors and continue
      }
    }
    
    // Move up one directory
    currentPath = path.dirname(currentPath);
  }
  
  // If no WORKSPACE file found, use the directory of the BUILD.bazel file itself
  return path.dirname(startPath);
}

// Bazel-related types and functions
export interface BazelTarget {
  name: string;
  type: "library" | "test" | "binary";
  buildLabel: string;
  testLabel?: string;
  deps: string[];
}

export interface BazelPackage {
  name: string;
  path: string;
  targets: BazelTarget[];
}

/**
 * Parse a BUILD.bazel file and extract dd_ios_package targets
 */
export async function parseBazelBuildFile(buildFilePath: string): Promise<BazelPackage | null> {
  // Validate input
  if (!buildFilePath || typeof buildFilePath !== 'string' || buildFilePath.length === 0) {
    return null;
  }
  
  // Check for suspicious path patterns
  if (buildFilePath.startsWith('@') || buildFilePath.includes('undefined') || buildFilePath.includes('null')) {
    return null;
  }
  
  try {
    const fs = await import("node:fs/promises");
    
    // Check if file exists first
    try {
      await fs.access(buildFilePath);
    } catch (accessError) {
      return null;
    }
    
    const content = await fs.readFile(buildFilePath, "utf-8");
    
    // Extract the package name from the path (parent directory of BUILD.bazel)
    const packagePath = path.dirname(buildFilePath);
    const packageName = path.basename(packagePath);
    
    // Find different types of Bazel iOS rules in the file
    const targets: BazelTarget[] = [];
    
    // 1. Try dd_ios_package rules (original format) - more flexible regex
    const ddPackageRegex = /dd_ios_package\s*\(\s*[\s\S]*?name\s*=\s*"([^"]+)"[\s\S]*?targets\s*=\s*\[([\s\S]*?)\]\s*[\s\S]*?\)/g;
    let ddPackageMatch;
    
    while ((ddPackageMatch = ddPackageRegex.exec(content)) !== null) {
      const [, packageTargetName, targetsBlock] = ddPackageMatch;
      
      // Parse targets more carefully by finding balanced parentheses
      let targetCount = 0;
      
      // Find all target.xxx( patterns and parse each one
      const targetStartRegex = /target\.(\w+)\s*\(/g;
      let targetStartMatch;
      
      while ((targetStartMatch = targetStartRegex.exec(targetsBlock)) !== null) {
        const targetType = targetStartMatch[1];
        const startPos = targetStartMatch.index + targetStartMatch[0].length - 1; // Position of opening (
        
        // Find the matching closing parenthesis
        let parenCount = 1;
        let endPos = startPos + 1;
        
        while (endPos < targetsBlock.length && parenCount > 0) {
          if (targetsBlock[endPos] === '(') parenCount++;
          if (targetsBlock[endPos] === ')') parenCount--;
          endPos++;
        }
        
        if (parenCount === 0) {
          // Extract the content between the parentheses
          const targetContent = targetsBlock.substring(startPos + 1, endPos - 1);
          
          // Extract the name from the target content
          const nameMatch = targetContent.match(/name\s*=\s*"([^"]+)"/);
          
          if (nameMatch) {
            targetCount++;
            const targetName = nameMatch[1];
            
            // Extract dependencies if present - search in the target content
            const deps: string[] = [];
            const depsRegex = /deps\s*=\s*\[([\s\S]*?)\]/;
            const depsMatch = depsRegex.exec(targetContent);
            
            if (depsMatch) {
              const depsContent = depsMatch[1];
              const depRegex = /"([^"]+)"/g;
              let depMatch;
              while ((depMatch = depRegex.exec(depsContent)) !== null) {
                deps.push(depMatch[1]);
              }
            }
            
            // Create target with build and test labels
            const relativePath = path.relative(findBazelWorkspaceRoot(packagePath), packagePath);
            const buildLabel = `//${relativePath}:${targetName}`;
            
            targets.push({
              name: targetName,
              type: targetType as "library" | "test" | "binary",
              buildLabel,
              testLabel: targetType === "test" ? buildLabel : undefined,
              deps,
            });
          }
        }
      }
    }
    
    // 2. Try cx_module and dx_module rules (DoorDash specific formats)
    const moduleRegex = /(cx_module|dx_module)\s*\(\s*([\s\S]*?)\s*\)/g;
    let moduleMatch;
    
    while ((moduleMatch = moduleRegex.exec(content)) !== null) {
      const [, moduleType, moduleConfig] = moduleMatch;
      
      // For cx_module/dx_module, create a library target with the package name
      const relativePath = path.relative(findBazelWorkspaceRoot(packagePath), packagePath);
      const buildLabel = `//${relativePath}:${packageName}`;
      
      // Extract dependencies if present
      const deps: string[] = [];
      const depsRegex = /deps\s*=\s*\[([\s\S]*?)\]/;
      const depsMatch = depsRegex.exec(moduleConfig);
      
      if (depsMatch) {
        const depsContent = depsMatch[1];
        const depRegex = /"([^"]+)"/g;
        let depMatch;
        while ((depMatch = depRegex.exec(depsContent)) !== null) {
          deps.push(depMatch[1]);
        }
      }
      
      targets.push({
        name: packageName, // Use directory name as target name
        type: "library", // cx_module/dx_module are typically libraries
        buildLabel,
        testLabel: undefined, // Module rules typically don't include test targets
        deps,
      });
      
      // Also look for potential test targets
      const testRegex = /(test_deps|test_srcs|test_resources)\s*=/g;
      if (testRegex.test(moduleConfig)) {
        const testLabel = `//${relativePath}:${packageName}Tests`;
        targets.push({
          name: `${packageName}Tests`,
          type: "test", 
          buildLabel: testLabel,
          testLabel: testLabel,
          deps: [`:${packageName}`], // Depend on main target
        });
      }
    }
    
    // 3. Try xcodeproj and other iOS-related rules
    const xcodeprojRegex = /(xcodeproj|top_level_target|ios_application|ios_framework|swift_library|ios_unit_test|ios_ui_test)\s*\(\s*[\s\S]*?name\s*=\s*"([^"]+)"[\s\S]*?\)/g;
    let xcodeprojMatch;
    
    while ((xcodeprojMatch = xcodeprojRegex.exec(content)) !== null) {
      const [, ruleType, ruleName] = xcodeprojMatch;
      
      const relativePath = path.relative(findBazelWorkspaceRoot(packagePath), packagePath);
      const buildLabel = `//${relativePath}:${ruleName}`;
      
      let targetType: "library" | "test" | "binary";
      if (ruleType === "ios_application" || ruleType === "xcodeproj") {
        targetType = "binary";
      } else if (ruleType.includes("test")) {
        targetType = "test"; 
      } else {
        targetType = "library";
      }
      
      targets.push({
        name: ruleName,
        type: targetType,
        buildLabel,
        testLabel: targetType === "test" ? buildLabel : undefined,
        deps: [], // Could extract deps if needed
      });
    }
    
    // 4. Generic target extraction - look for any named targets as fallback
    if (targets.length === 0) {
      // Look for any rule with a name parameter - more flexible patterns
      const genericPatterns = [
        /([a-zA-Z]\w*)\s*\(\s*[\s\S]*?name\s*=\s*"([^"]+)"[\s\S]*?\)/g,  // Standard pattern
        /([a-zA-Z]\w*)\s*\(\s*name\s*=\s*"([^"]+)"[\s\S]*?\)/g,          // Name first pattern
      ];
      
      for (const pattern of genericPatterns) {
        let genericMatch;
        let genericCount = 0;
        
        while ((genericMatch = pattern.exec(content)) !== null && genericCount < 10) { // Increased limit
          const [, ruleType, ruleName] = genericMatch;
          
          // Skip load statements and other non-target rules
          if (ruleType === "load" || 
              ruleType.startsWith("@") || 
              ruleType === "glob" ||
              ruleType === "select" ||
              ruleType === "config_setting" ||
              ruleType === "filegroup" ||
              ruleType.length < 3) {
            continue;
          }
          
          const relativePath = path.relative(findBazelWorkspaceRoot(packagePath), packagePath);
          const buildLabel = `//${relativePath}:${ruleName}`;
          
          // Try to infer type from rule name or target name
          let targetType: "library" | "test" | "binary";
          if (ruleType.includes("test") || ruleName.includes("Test") || ruleName.includes("test")) {
            targetType = "test";
          } else if (ruleType.includes("app") || ruleType.includes("binary") || ruleName.includes("App")) {
            targetType = "binary";
          } else {
            targetType = "library";
          }
          
          // Avoid duplicates
          const existing = targets.find(t => t.name === ruleName);
          if (!existing) {
            targets.push({
              name: ruleName,
              type: targetType,
              buildLabel,
              testLabel: targetType === "test" ? buildLabel : undefined,
              deps: [],
            });
            
            genericCount++;
          }
        }
        
        // If we found targets with this pattern, stop trying other patterns
        if (targets.length > 0) {
          break;
        }
      }
    }
    
    // 5. If still no targets after ALL parsing attempts, create default targets
    if (targets.length === 0) {
      const relativePath = path.relative(findBazelWorkspaceRoot(packagePath), packagePath);
      const buildLabel = `//${relativePath}:${packageName}`;
      
      // Create a generic library target using the directory name
      targets.push({
        name: packageName,
        type: "library",
        buildLabel,
        testLabel: undefined,
        deps: [],
      });
      
      // Also create a potential test target
      const testLabel = `//${relativePath}:${packageName}Tests`;
      targets.push({
        name: `${packageName}Tests`,
        type: "test", 
        buildLabel: testLabel,
        testLabel: testLabel,
        deps: [`:${packageName}`],
      });
    }
    
    return {
      name: packageName,
      path: packagePath,
      targets,
    };
  } catch (error) {
    commonLogger.warn("Failed to parse BUILD.bazel file", {
      buildFilePath,
      error,
    });
    return null;
  }
}

/**
 * Get all Bazel packages and their targets from the workspace
 */
export async function getBazelPackages(targetWorkspace?: string): Promise<BazelPackage[]> {
  // Use provided workspace or fall back to current workspace
  const workspace = targetWorkspace || getWorkspacePath();
  
  // NOTE: This function should only be used for bulk operations, not on-demand parsing
  console.log(`⚠️ getBazelPackages called - this should be rare! Use getCachedBazelPackage for individual files`);
  
  // Find all BUILD.bazel files
  const buildFiles = await findFilesRecursive({
    directory: workspace,
    depth: 10, // Allow deeper search for Bazel projects
    matcher: (file) => {
      return file.name === "BUILD.bazel" || file.name === "BUILD";
    },
  });
  
  console.log(`  - Found ${buildFiles.length} BUILD files (parsing all - slow!)`);
  
  const packages: BazelPackage[] = [];
  
  for (const buildFile of buildFiles) {
    const bazelPackage = await parseBazelBuildFile(buildFile);
    if (bazelPackage) {
      packages.push(bazelPackage);
    }
  }
  
  return packages;
}
