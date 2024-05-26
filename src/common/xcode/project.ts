import { XcodeWorkspaceFileRef } from "./workspace";
import { XcodeProject as XcodeProjectParser } from "@bacons/xcode";
import path from "path";
import { findFiles, findFilesRecursive, isFileExists } from "../files";

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

export class XcodeProject {
  private parser: XcodeProjectParser;
  // path to .xcodeproj (not .pbxproj)
  public projectPath: string;

  private constructor(options: { parser: XcodeProjectParser; projectPath: string }) {
    this.parser = options.parser;
    this.projectPath = options.projectPath;
  }

  getConfigurations(): string[] {
    const configurationList = this.parser.rootObject.props.buildConfigurationList;
    return configurationList.props.buildConfigurations.map((config: any) => config.props.name);
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
          })
        )
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

  static async parseProject(projectPath: string): Promise<XcodeProject> {
    const parser = XcodeProjectParser.open(path.join(projectPath, "project.pbxproj"));
    return new XcodeProject({
      parser: parser,
      projectPath: projectPath,
    });
  }

  static async fromFileRef(fileRef: XcodeWorkspaceFileRef): Promise<XcodeProject | null> {
    const projectPath = fileRef.getProjectPath();

    if (!projectPath) {
      return null;
    }

    return XcodeProject.parseProject(projectPath);
  }
}
