import path from "node:path";
import { XcodeProject as XcodeProjectParsed } from "@bacons/xcode";
import { type XcodeProject as XcodeProjectRaw, parse as parseChevrotain } from "@bacons/xcode/json";
import { findFiles, findFilesRecursive, isFileExists, readTextFile } from "../files";
import { uniqueFilter } from "../helpers";

class XcodeScheme {
  public name: string;
  public path: string;
  public project: XcodeProject;

  constructor(options: { name: string; path: string; project: XcodeProject }) {
    this.name = options.name;
    this.path = options.path;
    this.project = options.project;
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
}

export interface XcodeProject {
  projectPath: string;
  getConfigurations(): string[];
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

  /**
   * Start looking for ".xcscheme" in "xcschemes" directory.
   * It's common function for searching both shared and user-specific schemes
   */
  async findSchemes(startPath: string): Promise<XcodeScheme[]> {
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
            project: this,
          }),
        ),
      );
    }
    return schemes;
  }

  async getSchemes(): Promise<XcodeScheme[]> {
    const schemes = [];

    // Find shared schemes:
    // Ex: <projectPath>/xcshareddata/xcschemes/*.xcscheme
    const sharedSchemasDir = path.join(this.projectPath, "xcshareddata");
    schemes.push(...(await this.findSchemes(sharedSchemasDir)));

    // Then try to find user-specific schemes:
    // Ex: <projectPath>/xcuserdata/<username>.xcuserdatad/xcschemes/*.xcscheme
    const userDataDir = path.join(this.projectPath, "xcuserdata");
    const specificUserDataDir = await findFiles({
      directory: userDataDir,
      matcher: (file) => {
        return file.isDirectory() && file.name.endsWith(".xcuserdatad");
      },
    });
    if (specificUserDataDir.length > 0) {
      for (const dir of specificUserDataDir) {
        schemes.push(...(await this.findSchemes(dir)));
      }
    }

    // Provide default scheme if no schemes found
    if (schemes.length === 0) {
      // ex: "/path/to/MyApp.xcodeproj" -> "MyApp"
      const name = path.basename(this.projectPath).replace(/\.xcodeproj$/, "");
      const defaultScheme = new XcodeScheme({
        name: name,
        path: "",
        project: this,
      });
      schemes.push(defaultScheme);
    }

    return schemes;
  }

  async getSchemasNames(): Promise<string[]> {
    const schemes = await this.getSchemes();
    return schemes.map((scheme) => scheme.name);
  }
}

export class XcodeProjectFallbackParser implements XcodeProject {
  // path to .xcodeproj (not .pbxproj)
  public projectPath: string;

  private parsed: Partial<XcodeProjectRaw>;

  constructor(options: {
    parsed: Partial<XcodeProjectRaw>;
    projectPath: string;
  }) {
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
