import * as path from "node:path";
import type { BazelParseResult, BazelTarget, BazelScheme, BazelXcodeConfiguration, BazelPackageInfo } from "./types";

/**
 * Functional Bazel BUILD file parser
 * Extracts xcschemes, xcode_configurations, targets, and test targets
 */
export class BazelParser {
  /**
   * Parse a BUILD file content and return structured data
   */
  static parse(content: string, filePath?: string): BazelParseResult {
    const result: BazelParseResult = {
      xcschemes: [],
      xcode_configurations: [],
      targets: [],
      targetsTest: [],
    };

    // Parse different rule types
    result.xcschemes = this.parseSchemes(content);
    result.xcode_configurations = this.parseXcodeConfigurations(content);

    const { targets, testTargets } = this.parseTargets(content, filePath);
    result.targets = targets;
    result.targetsTest = testTargets;

    return result;
  }

  /**
   * Parse xcschemes from xcodeproj rules
   */
  private static parseSchemes(content: string): BazelScheme[] {
    const schemes: BazelScheme[] = [];

    // Find xcschemes arrays in xcodeproj rules using balanced parentheses
    const xcodeprojStartRegex = /xcodeproj\s*\(/g;
    let xcodeprojStartMatch;

    while ((xcodeprojStartMatch = xcodeprojStartRegex.exec(content)) !== null) {
      const startPos = xcodeprojStartMatch.index + xcodeprojStartMatch[0].length - 1; // Position of opening (
      const xcodeprojContent = BazelParserUtils.findBalancedParens(content, startPos);
      if (!xcodeprojContent) continue;

      // Extract xcschemes array using balanced bracket matching
      const xcschemesStartMatch = /xcschemes\s*=\s*\[/.exec(xcodeprojContent);
      if (!xcschemesStartMatch) continue;

      const schemesStartPos = xcschemesStartMatch.index + xcschemesStartMatch[0].length - 1; // Position of opening [
      const schemesContent = BazelParserUtils.findBalancedBrackets(xcodeprojContent, schemesStartPos);

      if (schemesContent) {
        // Parse doordash_scheme calls
        const doordashSchemeStartRegex = /doordash_scheme\s*\(/g;
        let doordashSchemeMatch;

        while ((doordashSchemeMatch = doordashSchemeStartRegex.exec(schemesContent)) !== null) {
          const schemeStartPos = doordashSchemeMatch.index + doordashSchemeMatch[0].length - 1;
          const schemeParams = BazelParserUtils.findBalancedParens(schemesContent, schemeStartPos);
          if (!schemeParams) continue;
          const nameMatch = /name\s*=\s*"([^"]+)"/.exec(schemeParams);
          const envMatch = /run_env\s*=\s*\{([\s\S]*?)\}/.exec(schemeParams);

          if (nameMatch) {
            const env: Record<string, string> = {};
            if (envMatch) {
              const envContent = envMatch[1];
              const envPairs = envContent.match(/"([^"]+)"\s*:\s*"([^"]+)"/g);
              if (envPairs) {
                envPairs.forEach((pair) => {
                  const [, key, value] = pair.match(/"([^"]+)"\s*:\s*"([^"]+)"/) || [];
                  if (key && value) env[key] = value;
                });
              }
            }

            schemes.push({
              name: nameMatch[1],
              type: "doordash_scheme",
              buildTargets: [],
              env,
            });
          }
        }

        // Parse doordash_appclip_scheme calls
        const appclipSchemeStartRegex = /doordash_appclip_scheme\s*\(/g;
        let appclipSchemeMatch;

        while ((appclipSchemeMatch = appclipSchemeStartRegex.exec(schemesContent)) !== null) {
          const appclipStartPos = appclipSchemeMatch.index + appclipSchemeMatch[0].length - 1;
          const schemeParams = BazelParserUtils.findBalancedParens(schemesContent, appclipStartPos);
          if (!schemeParams) continue;
          const nameMatch = /name\s*=\s*"([^"]+)"/.exec(schemeParams);

          if (nameMatch) {
            schemes.push({
              name: nameMatch[1],
              type: "doordash_appclip_scheme",
              buildTargets: [],
            });
          }
        }

        // Parse xcschemes.scheme calls
        const xcschemesSchemeStartRegex = /xcschemes\.scheme\s*\(/g;
        let xcschemeSchemeMatch;

        while ((xcschemeSchemeMatch = xcschemesSchemeStartRegex.exec(schemesContent)) !== null) {
          const xcschemeStartPos = xcschemeSchemeMatch.index + xcschemeSchemeMatch[0].length - 1;
          const schemeParams = BazelParserUtils.findBalancedParens(schemesContent, xcschemeStartPos);
          if (!schemeParams) continue;
          const nameMatch = /name\s*=\s*"([^"]+)"/.exec(schemeParams);

          if (nameMatch) {
            const runMatch = /run\s*=\s*xcschemes\.run\s*\(\s*([\s\S]*?)\s*\)/.exec(schemeParams);
            let buildTargets: string[] = [];
            let launchTarget: string | undefined;
            let env: Record<string, string> = {};

            if (runMatch) {
              const runContent = runMatch[1];

              // Extract build_targets
              const buildTargetsMatch = /build_targets\s*=\s*\[([\s\S]*?)\]/.exec(runContent);
              if (buildTargetsMatch) {
                const buildTargetsContent = buildTargetsMatch[1];
                const targets = buildTargetsContent.match(/"([^"]+)"/g);
                if (targets) {
                  buildTargets = targets.map((t) => t.replace(/"/g, ""));
                }
              }

              // Extract launch_target
              const launchTargetMatch = /launch_target\s*=\s*"([^"]+)"/.exec(runContent);
              if (launchTargetMatch) {
                launchTarget = launchTargetMatch[1];
              }

              // Extract env
              const envMatch = /env\s*=\s*\{([\s\S]*?)\}/.exec(runContent);
              if (envMatch) {
                const envContent = envMatch[1];
                const envPairs = envContent.match(/"([^"]+)"\s*:\s*"([^"]+)"/g);
                if (envPairs) {
                  envPairs.forEach((pair) => {
                    const [, key, value] = pair.match(/"([^"]+)"\s*:\s*"([^"]+)"/) || [];
                    if (key && value) env[key] = value;
                  });
                }
              }
            }

            schemes.push({
              name: nameMatch[1],
              type: "xcschemes_scheme",
              buildTargets,
              launchTarget,
              env,
            });
          }
        }
      }
    }

    return schemes;
  }

  /**
   * Parse xcode_configurations from load statements or config rules
   */
  private static parseXcodeConfigurations(content: string): BazelXcodeConfiguration[] {
    const configurations: BazelXcodeConfiguration[] = [];

    // Look for xcode_configurations references in load statements
    const configLoadRegex = /load\s*\(\s*"([^"]+)"\s*,\s*"xcode_configurations"\s*\)/g;
    let configMatch;

    while ((configMatch = configLoadRegex.exec(content)) !== null) {
      const configFile = configMatch[1];
      configurations.push({
        name: "xcode_configurations",
        buildSettings: {
          source: configFile,
        },
      });
    }

    // Look for direct xcode_configurations usage
    const directConfigRegex = /xcode_configurations\s*=\s*([\w_]+)/g;
    let directMatch;

    while ((directMatch = directConfigRegex.exec(content)) !== null) {
      const configName = directMatch[1];
      if (!configurations.some((c) => c.name === configName)) {
        configurations.push({
          name: configName,
        });
      }
    }

    return configurations;
  }

  /**
   * Parse targets from different rule types (dd_ios_package, cx_module, etc.)
   */
  private static parseTargets(
    content: string,
    filePath?: string,
  ): { targets: BazelTarget[]; testTargets: BazelTarget[] } {
    const targets: BazelTarget[] = [];
    const testTargets: BazelTarget[] = [];

    // Parse dd_ios_package targets
    this.parseDdIosPackageTargets(content, filePath, targets, testTargets);

    // Parse cx_module targets
    this.parseCxModuleTargets(content, filePath, targets, testTargets);

    // Parse swift_library targets
    this.parseSwiftLibraryTargets(content, filePath, targets);

    // Parse dd_ios_application targets
    this.parseDdIosApplicationTargets(content, filePath, targets);

    // Parse top_level_target entries from xcodeproj
    this.parseTopLevelTargets(content, filePath, targets, testTargets);

    return { targets, testTargets };
  }

  /**
   * Parse targets from dd_ios_package rules
   */
  private static parseDdIosPackageTargets(
    content: string,
    filePath: string | undefined,
    targets: BazelTarget[],
    testTargets: BazelTarget[],
  ): void {
    const ddIosPackageStartRegex = /dd_ios_package\s*\(/g;
    let packageStartMatch;

    while ((packageStartMatch = ddIosPackageStartRegex.exec(content)) !== null) {
      const startPos = packageStartMatch.index + packageStartMatch[0].length - 1; // Position of opening (
      const packageContent = BazelParserUtils.findBalancedParens(content, startPos);
      if (!packageContent) continue;

      // Extract package name
      const packageNameMatch = /name\s*=\s*"([^"]+)"/.exec(packageContent);
      const packageName = packageNameMatch?.[1] || "UnknownPackage";

      // Extract targets array with proper bracket matching
      const targetsStartMatch = /targets\s*=\s*\[/.exec(packageContent);
      if (!targetsStartMatch) continue;

      const bracketsStartPos = targetsStartMatch.index + targetsStartMatch[0].length - 1; // Position of opening [
      const targetsContent = BazelParserUtils.findBalancedBrackets(packageContent, bracketsStartPos);
      if (!targetsContent) continue;

      // Parse target.library() calls
      const libraryRegex = /target\.library\s*\(\s*([\s\S]*?)\s*\)/g;
      let libraryMatch;

      while ((libraryMatch = libraryRegex.exec(targetsContent)) !== null) {
        const targetParams = libraryMatch[1];
        const target = this.parseTargetParams(targetParams, "library", packageName, filePath);
        if (target) targets.push(target);
      }

      // Parse target.test() calls
      const testRegex = /target\.test\s*\(\s*([\s\S]*?)\s*\)/g;
      let testMatch;

      while ((testMatch = testRegex.exec(targetsContent)) !== null) {
        const targetParams = testMatch[1];
        const target = this.parseTargetParams(targetParams, "test", packageName, filePath);
        if (target) testTargets.push(target);
      }

      // Parse target.binary() calls
      const binaryRegex = /target\.binary\s*\(\s*([\s\S]*?)\s*\)/g;
      let binaryMatch;

      while ((binaryMatch = binaryRegex.exec(targetsContent)) !== null) {
        const targetParams = binaryMatch[1];
        const target = this.parseTargetParams(targetParams, "binary", packageName, filePath);
        if (target) targets.push(target);
      }
    }
  }

  /**
   * Parse cx_module targets (creates default library and test targets)
   */
  private static parseCxModuleTargets(
    content: string,
    filePath: string | undefined,
    targets: BazelTarget[],
    testTargets: BazelTarget[],
  ): void {
    const cxModuleRegex = /cx_module\s*\(\s*([\s\S]*?)\s*\)/g;
    let moduleMatch;

    while ((moduleMatch = cxModuleRegex.exec(content)) !== null) {
      const moduleContent = moduleMatch[1];

      // For cx_module, create a default library target using directory name
      const packageDir = filePath ? path.dirname(filePath) : "";
      const packageName = path.basename(packageDir) || "CxModule";

      const packagePath = this.getPackagePath(filePath);
      const buildLabel = `//${packagePath}:${packageName}`;

      // Create library target
      targets.push({
        name: packageName,
        type: "library",
        deps: [],
        buildLabel,
        path: packageDir,
      });

      // Create corresponding test target
      testTargets.push({
        name: `${packageName}Tests`,
        type: "test",
        deps: [`:${packageName}`],
        buildLabel: `//${packagePath}:${packageName}Tests`,
        testLabel: `//${packagePath}:${packageName}Tests`,
        path: `${packageDir}/Tests`,
      });
    }
  }

  /**
   * Parse top_level_target entries from xcodeproj rules
   */
  private static parseTopLevelTargets(
    content: string,
    filePath: string | undefined,
    targets: BazelTarget[],
    testTargets: BazelTarget[],
  ): void {
    const topLevelRegex = /top_level_target\s*\(\s*"([^"]+)"\s*[,)]?/g;
    let targetMatch;

    while ((targetMatch = topLevelRegex.exec(content)) !== null) {
      const targetLabel = targetMatch[1];
      const targetName = targetLabel.split(":").pop() || targetLabel;

      // Determine if it's a test target
      const isTest = targetName.toLowerCase().includes("test") || targetLabel.toLowerCase().includes("/tests");

      const target: BazelTarget = {
        name: targetName,
        type: isTest ? "test" : "binary",
        deps: [],
        buildLabel: targetLabel,
        testLabel: isTest ? targetLabel : undefined,
      };

      if (isTest) {
        testTargets.push(target);
      } else {
        targets.push(target);
      }
    }
  }

  /**
   * Parse swift_library targets
   */
  private static parseSwiftLibraryTargets(content: string, filePath: string | undefined, targets: BazelTarget[]): void {
    const swiftLibraryRegex = /swift_library\s*\(/g;
    let libraryMatch;

    while ((libraryMatch = swiftLibraryRegex.exec(content)) !== null) {
      const startPos = libraryMatch.index + libraryMatch[0].length - 1; // Position of opening (
      const libraryContent = BazelParserUtils.findBalancedParens(content, startPos);
      if (!libraryContent) continue;

      // Extract name
      const nameMatch = /name\s*=\s*"([^"]+)"/.exec(libraryContent);
      if (!nameMatch) continue;

      const targetName = nameMatch[1];

      // Extract deps
      const deps: string[] = [];
      const depsMatch = /deps\s*=\s*\[([\s\S]*?)\]/gm.exec(libraryContent);
      if (depsMatch) {
        const depsList = BazelParserUtils.extractStringArray(`[${depsMatch[1]}]`);
        deps.push(...depsList);
      }

      // Create build label
      const packagePath = this.getPackagePath(filePath);
      const buildLabel = `//${packagePath}:${targetName}`;

      const target: BazelTarget = {
        name: targetName,
        type: "library",
        deps,
        buildLabel,
      };

      targets.push(target);
    }
  }

  /**
   * Parse dd_ios_application targets
   */
  private static parseDdIosApplicationTargets(
    content: string,
    filePath: string | undefined,
    targets: BazelTarget[],
  ): void {
    const ddIosApplicationRegex = /dd_ios_application\s*\(/g;
    let appMatch;

    while ((appMatch = ddIosApplicationRegex.exec(content)) !== null) {
      const startPos = appMatch.index + appMatch[0].length - 1; // Position of opening (
      const appContent = BazelParserUtils.findBalancedParens(content, startPos);
      if (!appContent) continue;

      // Extract name
      const nameMatch = /name\s*=\s*"([^"]+)"/.exec(appContent);
      if (!nameMatch) continue;

      const targetName = nameMatch[1];

      // Extract deps
      const deps: string[] = [];
      const depsMatch = /deps\s*=\s*\[([\s\S]*?)\]/gm.exec(appContent);
      if (depsMatch) {
        const depsList = BazelParserUtils.extractStringArray(`[${depsMatch[1]}]`);
        deps.push(...depsList);
      }

      // Create build label
      const packagePath = this.getPackagePath(filePath);
      const buildLabel = `//${packagePath}:${targetName}`;

      const target: BazelTarget = {
        name: targetName,
        type: "binary",
        deps,
        buildLabel,
      };

      targets.push(target);
    }
  }

  /**
   * Parse target parameters (name, deps, path, resources)
   */
  private static parseTargetParams(
    params: string,
    type: "library" | "test" | "binary",
    packageName: string,
    filePath?: string,
  ): BazelTarget | null {
    // Extract name
    const nameMatch = /name\s*=\s*"([^"]+)"/.exec(params);
    if (!nameMatch) return null;

    const targetName = nameMatch[1];

    // Extract deps
    const deps: string[] = [];
    const depsMatch = /deps\s*=\s*\[([\s\S]*?)\]/.exec(params);
    if (depsMatch) {
      const depsContent = depsMatch[1];
      const depMatches = depsContent.match(/"([^"]+)"/g);
      if (depMatches) {
        deps.push(...depMatches.map((d) => d.replace(/"/g, "")));
      }
    }

    // Extract path
    const pathMatch = /path\s*=\s*"([^"]+)"/.exec(params);
    const targetPath = pathMatch?.[1];

    // Extract resources
    const resources: string[] = [];
    const resourcesMatch = /resources\s*=\s*\[([\s\S]*?)\]/.exec(params);
    if (resourcesMatch) {
      const resourcesContent = resourcesMatch[1];
      const resourceMatches = resourcesContent.match(/"([^"]+)"/g);
      if (resourceMatches) {
        resources.push(...resourceMatches.map((r) => r.replace(/"/g, "")));
      }
    }

    // Build labels
    const packagePath = this.getPackagePath(filePath);
    const buildLabel = `//${packagePath}:${targetName}`;
    const testLabel = type === "test" ? buildLabel : undefined;

    return {
      name: targetName,
      type,
      deps,
      path: targetPath,
      resources: resources.length > 0 ? resources : undefined,
      buildLabel,
      testLabel,
    };
  }

  /**
   * Get package path from file path (relative to workspace root)
   */
  private static getPackagePath(filePath?: string): string {
    if (!filePath) return "";

    // Remove BUILD file and get directory path
    const dir = path.dirname(filePath);

    // Try to extract a reasonable package path from the directory structure
    // Look for common Bazel project patterns and extract the full package path

    if (dir.includes("/Apps/")) {
      // Extract from '/Apps/' onward (e.g., /path/to/Apps/Consumer/SomeModule -> Apps/Consumer/SomeModule)
      const appsIndex = dir.indexOf("/Apps/");
      return dir.substring(appsIndex + 1); // Remove leading '/'
    } else if (dir.includes("/Packages/")) {
      // Extract from '/Packages/' onward (e.g., /path/to/Packages/DoordashAttestation -> Packages/DoordashAttestation)
      const packagesIndex = dir.indexOf("/Packages/");
      return dir.substring(packagesIndex + 1); // Remove leading '/'
    } else if (dir.includes("/Libraries/")) {
      // Extract from '/Libraries/' onward
      const librariesIndex = dir.indexOf("/Libraries/");
      return dir.substring(librariesIndex + 1); // Remove leading '/'
    } else if (dir.includes("/Sources/")) {
      // For SPM-style structure, try to get the package name before /Sources/
      const sourcesIndex = dir.indexOf("/Sources/");
      const beforeSources = dir.substring(0, sourcesIndex);
      const packageName = path.basename(beforeSources);
      return packageName;
    } else {
      // For simple cases or when no known patterns are found,
      // try to extract the last meaningful directory name
      // If it's a deep path like /long/path/to/MyPackage, return MyPackage
      const segments = dir.split(path.sep).filter((seg) => seg.length > 0);
      if (segments.length > 0) {
        // For paths like /workspace/Packages/DoordashAttestation, we want Packages/DoordashAttestation
        if (segments.length >= 2 && segments[segments.length - 2] === "Packages") {
          return `Packages/${segments[segments.length - 1]}`;
        }
        // Just use the last segment for simple cases
        return segments[segments.length - 1];
      }
      return "";
    }
  }

  /**
   * Parse multiple files and combine results
   */
  static parsePackage(buildFileContent: string, packagePath: string): BazelPackageInfo {
    const packageName = path.basename(packagePath);
    const parseResult = this.parse(buildFileContent, path.join(packagePath, "BUILD.bazel"));

    return {
      name: packageName,
      path: packagePath,
      parseResult,
    };
  }
}

/**
 * Utility functions for parsing
 */
export const BazelParserUtils = {
  /**
   * Extract string literals from Bazel array syntax
   */
  extractStringArray(content: string): string[] {
    const matches = content.match(/"([^"]+)"/g);
    return matches ? matches.map((m) => m.replace(/"/g, "")) : [];
  },

  /**
   * Extract key-value pairs from Bazel dict syntax
   */
  extractDict(content: string): Record<string, string> {
    const result: Record<string, string> = {};
    const pairs = content.match(/"([^"]+)"\s*:\s*"([^"]+)"/g);

    if (pairs) {
      pairs.forEach((pair) => {
        const match = pair.match(/"([^"]+)"\s*:\s*"([^"]+)"/);
        if (match) {
          result[match[1]] = match[2];
        }
      });
    }

    return result;
  },

  /**
   * Find balanced parentheses content
   */
  findBalancedParens(content: string, startPos: number): string | null {
    let parenCount = 0;
    let pos = startPos;

    while (pos < content.length) {
      if (content[pos] === "(") parenCount++;
      if (content[pos] === ")") parenCount--;
      if (parenCount === 0 && pos > startPos) {
        return content.substring(startPos + 1, pos);
      }
      pos++;
    }

    return null;
  },

  /**
   * Find balanced brackets content
   */
  findBalancedBrackets(content: string, startPos: number): string | null {
    let bracketCount = 0;
    let pos = startPos;

    while (pos < content.length) {
      if (content[pos] === "[") bracketCount++;
      if (content[pos] === "]") bracketCount--;
      if (bracketCount === 0 && pos > startPos) {
        return content.substring(startPos + 1, pos);
      }
      pos++;
    }

    return null;
  },
};
