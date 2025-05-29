import path from "node:path";
import { XcodeProject as XcodeProjectParsed } from "@bacons/xcode";
import { type XcodeProject as XcodeProjectRaw, parse as parseChevrotain } from "@bacons/xcode/json";
import { type XmlDocument, XmlElement, type XmlNode, parseXml } from "@rgrove/parse-xml";
import { findFiles, findFilesRecursive, isFileExists, readFile, readTextFile, statFile } from "../files";
import { uniqueFilter } from "../helpers";

function isXMLElement(obj: XmlNode): obj is XmlElement {
  return obj instanceof XmlElement;
}

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

  public _cache: XmlDocument | null = null;
  public _cacheModified: number | null = null;

  constructor(options: { name: string; path: string; project: XcodeProject }) {
    this.name = options.name;
    this.path = options.path;
    this.project = options.project;
  }

  async getXml(): Promise<XmlDocument | null> {
    // If scheme file doesn't exist, it means that default scheme settings should be used
    if (!this.path) {
      return null;
    }

    const stat = await statFile(this.path);
    if (this._cache && this._cacheModified && stat.mtimeMs === this._cacheModified) {
      return this._cache;
    }

    const content = await readFile(this.path);
    const contentString = content.toString();
    const contentParsed = parseXml(contentString);
    this._cache = contentParsed;
    this._cacheModified = stat.mtimeMs;
    return contentParsed;
  }

  /**
   * Get target name that should be used to lauch the app
   */
  async getTargetToLaunch(): Promise<string | null> {
    /** Example:
     * <Scheme ...>
     *  <BuildAction ...> ... </BuildAction>
     *  <TestAction ...> ... </TestAction>
     *  <LaunchAction ...>
     *   <BuildableProductRunnable ...>
     *    <!-- This is the target that will be launched -->
     *    <BuildableReference
     *     BuildableName = "MyApp.app"
     *     BlueprintName = "MyApp">
     *    </BuildableReference>
     *   </BuildableProductRunnable>
     *  </LaunchAction>
     *  <ProfileAction ...> ... </ProfileAction>
     *  <AnalyzeAction ...> ... </AnalyzeAction>
     *  <ArchiveAction ...> ... </ArchiveAction>
     * </Scheme>
     */
    const schemeContent = await this.getXml();

    // When there is no scheme file it means that it should use default target name
    if (!schemeContent) {
      return null;
    }

    const schemeRoot = schemeContent.root;
    if (!schemeRoot) {
      return null;
    }

    /**
     * <LaunchAction> configures the Run phase of the scheme (the action that launches the app).
     * In Xcode's UI this is labeled "Run" in the scheme editor, but in the file it's called
     * LaunchAction​. This element defines how the app or executable is launched when you run
     * the scheme.
     */
    const launchAction = schemeRoot.children.filter(isXMLElement).find((element) => element.name === "LaunchAction");
    if (!launchAction) {
      return null;
    }

    /**
     * <BuildableProductRunnable> – This child element appears in LaunchAction and represents
     * the executable to run. It contains the BuildableReference for the product that will be
     * launched. Typically, for an app scheme, there will be one BuildableProductRunnable
     * referencing the .app target.
     *
     * INFO: There are other types of runnable elements, such as RemoteRunnable and BuildableLocationRunnable,
     * but these are not currently supported by this parser. Please open an issue if you need support for these.
     */
    const buildableProductRunnable = launchAction.children
      .filter(isXMLElement)
      .find((element) => element.name === "BuildableProductRunnable");
    if (!buildableProductRunnable) {
      return null;
    }

    /**
     * <BuildableReference> is the glue between the scheme and the project's targets, ensuring the scheme
     * knows which target it's referring to. The BlueprintName attribute is the name of the target.
     *
     * This target is the one that will be launched when you run the scheme in Xcode.
     */
    const buildableReference = buildableProductRunnable.children
      .filter(isXMLElement)
      .find((element) => element.name === "BuildableReference");

    if (!buildableReference) {
      return null;
    }

    // BlueprintName == TargetName
    return buildableReference.attributes.BlueprintName || "";
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
    const sharedSchemasDir = path.join(project.projectPath, "xcshareddata");
    schemes.push(...(await XcodeScheme.findSchemes(project, sharedSchemasDir)));

    // Then try to find user-specific schemes:
    // Ex: <projectPath>/xcuserdata/<username>.xcuserdatad/xcschemes/*.xcscheme
    const userDataDir = path.join(project.projectPath, "xcuserdata");
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
