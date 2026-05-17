import path from "node:path";

import { XcodeProject as XcodeProjectParsed } from "@bacons/xcode";
import { type XcodeProject as XcodeProjectRaw, parse as parseChevrotain } from "@bacons/xcode/json";

import { findFiles, findFilesRecursive, isFileExists, readFile, readTextFile, statFile } from "../files";
import { uniqueFilter } from "../helpers";
import { SchemeDocument } from "./xcscheme";

export interface XcodeProject {
  projectPath: string;
  getConfigurations(): string[];
  getTargets(): string[];
  getSchemes(): Promise<XcodeScheme[]>;
  getScheme(name: string): Promise<XcodeScheme | null>;
}

export class XcodeScheme {
  public name: string;
  public path: string;
  public project: XcodeProject;

  #cache: SchemeDocument | null = null;
  #cacheModified: number | null = null;

  constructor(options: { name: string; path: string; project: XcodeProject }) {
    this.name = options.name;
    this.path = options.path;
    this.project = options.project;
  }

  /**
   * Parse the `.xcscheme` file into a typed SchemeDocument. Returns null if the
   * scheme has no on-disk path (in which case Xcode's default scheme settings
   * apply). Result is cached and invalidated by mtime.
   */
  async getScheme(): Promise<SchemeDocument | null> {
    if (!this.path) {
      return null;
    }
    const stat = await statFile(this.path);
    if (this.#cache && this.#cacheModified === stat.mtimeMs) {
      return this.#cache;
    }
    const content = await readFile(this.path);
    const doc = SchemeDocument.parse(content.toString());
    this.#cache = doc;
    this.#cacheModified = stat.mtimeMs;
    return doc;
  }

  /**
   * Name of the target that the scheme's Run action launches (the BlueprintName
   * of the BuildableProductRunnable inside <LaunchAction>). Returns null when
   * there is no scheme file, no LaunchAction, or only a RemoteRunnable /
   * MacroExpansion (e.g. framework schemes).
   */
  async getTargetToLaunch(): Promise<string | null> {
    const doc = await this.getScheme();
    return doc?.targetToLaunch() ?? null;
  }

  static fromFile(options: { schemePath: string; project: XcodeProject }): XcodeScheme {
    let name: string;
    const match = options.schemePath.match(/xcschemes\/(.+)\.xcscheme$/);
    if (match) {
      name = match[1];
    } else {
      // Fallback to basename if regexp doesn't match
      name = path.basename(options.schemePath, ".xcscheme");
    }

    return new XcodeScheme({
      name: name,
      path: options.schemePath,
      project: options.project,
    });
  }

  /**
   * Start looking for ".xcscheme" in "xcschemes" directory.
   * It's common function for searching both shared and user-specific schemes
   */
  static async findSchemes(project: XcodeProject, startPath: string): Promise<XcodeScheme[]> {
    const schemes = [];

    const schemesDir = path.join(startPath, "xcschemes");
    const schemesDirExists = await isFileExists(schemesDir);
    if (schemesDirExists) {
      const files = await findFilesRecursive({
        directory: schemesDir,
        depth: 4,
        matcher: (file) => file.name.endsWith(".xcscheme"),
      });

      schemes.push(
        ...files.map((file) =>
          XcodeScheme.fromFile({
            schemePath: file,
            project: project,
          }),
        ),
      );
    }
    return schemes;
  }

  static async getSchemes(project: XcodeProject): Promise<XcodeScheme[]> {
    const schemes = [];

    // Find shared schemes:
    // Ex: <projectPath>/xcshareddata/xcschemes/*.xcscheme
    const sharedSchemesDir = path.join(project.projectPath, "xcshareddata");
    schemes.push(...(await XcodeScheme.findSchemes(project, sharedSchemesDir)));

    // Then try to find user-specific schemes:
    // Ex: <projectPath>/xcuserdata/<username>.xcuserdatad/xcschemes/*.xcscheme
    const userDataDir = path.join(project.projectPath, "xcuserdata");
    const userDataDirExists = await isFileExists(userDataDir);
    if (userDataDirExists) {
      const specificUserDataDir = await findFiles({
        directory: userDataDir,
        matcher: (file) => {
          return file.isDirectory() && file.name.endsWith(".xcuserdatad");
        },
      });
      if (specificUserDataDir.length > 0) {
        for (const dir of specificUserDataDir) {
          schemes.push(...(await XcodeScheme.findSchemes(project, dir)));
        }
      }
    }

    // Provide default scheme if no schemes found
    if (schemes.length === 0) {
      // ex: "/path/to/MyApp.xcodeproj" -> "MyApp"
      const name = path.basename(project.projectPath).replace(/\.xcodeproj$/, "");
      const defaultScheme = new XcodeScheme({
        name: name,
        path: "",
        project: project,
      });
      schemes.push(defaultScheme);
    }

    return schemes;
  }

  static async getScheme(project: XcodeProject, name: string): Promise<XcodeScheme | null> {
    const schemes = await XcodeScheme.getSchemes(project);
    return schemes.find((scheme) => scheme.name === name) || null;
  }
}

export class XcodeProjectBaconParser implements XcodeProject {
  private parsed: XcodeProjectParsed;
  // path to .xcodeproj (not .pbxproj)
  public projectPath: string;

  constructor(options: { parsed: XcodeProjectParsed; projectPath: string }) {
    this.parsed = options.parsed;
    this.projectPath = options.projectPath;
  }

  getConfigurations(): string[] {
    const configurationList = this.parsed.rootObject.props.buildConfigurationList;
    return configurationList.props.buildConfigurations.map((config) => config.props?.name).filter((name) => !!name);
  }

  getTargets(): string[] {
    // todo: test it
    const targets = this.parsed.rootObject.props.targets;
    return targets.map((target) => target.props?.name).filter((name) => !!name);
  }

  async getSchemes(): Promise<XcodeScheme[]> {
    return await XcodeScheme.getSchemes(this);
  }

  async getScheme(name: string): Promise<XcodeScheme | null> {
    return await XcodeScheme.getScheme(this, name);
  }

  async getSchemesNames(): Promise<string[]> {
    const schemes = await this.getSchemes();
    return schemes.map((scheme) => scheme.name);
  }
}

export class XcodeProjectFallbackParser implements XcodeProject {
  // path to .xcodeproj (not .pbxproj)
  public projectPath: string;

  private parsed: Partial<XcodeProjectRaw>;

  constructor(options: { parsed: Partial<XcodeProjectRaw>; projectPath: string }) {
    this.parsed = options.parsed;
    this.projectPath = options.projectPath;
  }

  getConfigurations(): string[] {
    const objects = Object.values(this.parsed.objects ?? {});
    return objects
      .filter((obj) => obj.isa === "XCBuildConfiguration")
      .map((obj: any) => obj.name ?? null)
      .filter((name) => name !== null)
      .filter(uniqueFilter);
  }

  getTargets(): string[] {
    // todo: test it
    const objects = Object.values(this.parsed.objects ?? {});
    return objects
      .filter((obj) => obj.isa === "PBXNativeTarget")
      .map((obj: any) => obj.name ?? null)
      .filter((name) => name !== null)
      .filter(uniqueFilter);
  }

  async getSchemes(): Promise<XcodeScheme[]> {
    return await XcodeScheme.getSchemes(this);
  }

  async getScheme(name: string): Promise<XcodeScheme | null> {
    return await XcodeScheme.getScheme(this, name);
  }
}

export async function parseXcodeProject(projectPath: string): Promise<XcodeProject> {
  const pbxprojPath = path.join(projectPath, "project.pbxproj");
  try {
    const parsed = XcodeProjectParsed.open(pbxprojPath);
    return new XcodeProjectBaconParser({
      parsed: parsed,
      projectPath: projectPath,
    });
  } catch (error) {
    const projectRaw = await readTextFile(pbxprojPath);
    const parsed = parseChevrotain(projectRaw);
    return new XcodeProjectFallbackParser({
      parsed: parsed,
      projectPath: projectPath,
    });
  }
}
