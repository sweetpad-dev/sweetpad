import path from "node:path";
import { XmlElement, type XmlNode, parseXml } from "@rgrove/parse-xml";
import { readFile } from "../files";
import { commonLogger } from "../logger";
import { assertUnreachable, isNotNull } from "../types";
import { type XcodeProject, type XcodeScheme, parseXcodeProject } from "./project";

const XCODE_WORKSPACE_LOCATIONS_TYPES = ["self", "container", "group", "developer", "absolute"] as const;
type XcodeWorkspaceLocationType = (typeof XCODE_WORKSPACE_LOCATIONS_TYPES)[number];

interface IXcodeWorksaceItem {
  type: "group" | "fileref";
}

export class XcodeWorkspaceFileRef implements IXcodeWorksaceItem {
  public readonly type = "fileref" as const;
  public location: XcodeWorkspaceLocation;

  constructor(options: { location: XcodeWorkspaceLocation }) {
    this.location = options.location;
  }
}

function isXMLElement(obj: XmlNode): obj is XmlElement {
  return obj instanceof XmlElement;
}

function parseLocation(location: string): XcodeWorkspaceLocation {
  const [rawObj, path] = location.split(":", 2);
  const obj = rawObj as XcodeWorkspaceLocationType;

  return {
    obj: obj,
    path: path,
  };
}

export class XcodeWorkspaceGroup implements IXcodeWorksaceItem {
  public readonly type = "group" as const;
  public name: string | undefined;
  public location: XcodeWorkspaceLocation | undefined;
  public children: XcodeWorksaceItem[];

  constructor(options: {
    name: string | undefined;
    location: XcodeWorkspaceLocation | undefined;
    children: XcodeWorksaceItem[];
  }) {
    this.name = options.name;
    this.location = options.location;
    this.children = options.children;
  }
}

export type XcodeWorksaceItem = XcodeWorkspaceFileRef | XcodeWorkspaceGroup;

type XcodeWorkspaceLocation = {
  obj: XcodeWorkspaceLocationType;
  path: string;
};

/**
 * WIP: I've added that class when I was on parsing xcode state from files,
 * but I didn't finish it
 */
export class XcodeWorkspace {
  // path to ".xcworkspace" (not "contents.xcworkspacedata", which is inside of this directory)
  public xcworkspacePath: string;

  // Items represent tree structure of workspace:
  // - FileRef: reference to xcodeproject or other files
  // - Group: group of FileRefs or other groups
  public children: XcodeWorksaceItem[];

  /**
   * Directory where ".xcworkspace" is located
   */
  get xcworkspaceParentPath(): string {
    return path.dirname(this.xcworkspacePath);
  }

  constructor(options: { xcworkspacePath: string; children: XcodeWorksaceItem[] }) {
    this.xcworkspacePath = options.xcworkspacePath;
    this.children = options.children;
  }

  async parseProject(projectPath: string): Promise<XcodeProject | null> {
    try {
      return await parseXcodeProject(projectPath);
    } catch (error) {
      commonLogger.error("Failed to parse xcode project", {
        error: error,
        projectPath: projectPath,
      });
      return null;
    }
  }

  /**
   * Get all projects from workspace as flat list (unlike tree structure in workspace)
   */
  async getProjects(): Promise<XcodeProject[]> {
    const projects: XcodeProject[] = []; // doing illegal things, but i'm ok 🚨
    await this.getGroupChildrenProjects({
      ancestors: [],
      children: this.children,
      projects: projects,
    });
    return projects;
  }

  async getScheme(options: {
    name: string;
  }): Promise<XcodeScheme | null> {
    const projects = await this.getProjects();
    for (const project of projects) {
      const scheme = await project.getScheme(options.name);
      if (scheme) {
        return scheme;
      }
    }
    return null;
  }

  /**
   * Get projects from children of the group
   */
  async getGroupChildrenProjects(options: {
    ancestors: XcodeWorkspaceGroup[];
    children: XcodeWorksaceItem[];
    projects: XcodeProject[];
  }): Promise<void> {
    const { ancestors, children, projects } = options;

    for (const child of children) {
      if (child.type === "group") {
        // With new group we need to go deeper by calling this function again
        const group = child;
        const newAncestors = [...ancestors, group];
        await this.getGroupChildrenProjects({
          ancestors: newAncestors,
          children: group.children,
          projects: projects,
        });
      } else if (child.type === "fileref") {
        // FileRef is actully can be a reference to project, so we need to resolve it
        const fileRef = child;
        const projectPath = await this.resolveProjectPath(ancestors, fileRef);
        commonLogger.debug("Resolved project path", { projectPath: projectPath });
        if (!projectPath) continue;

        if (projects.some((proj) => proj.projectPath === projectPath)) {
          commonLogger.debug("Project is duplicated", { projectPath: projectPath });
          continue;
        }

        const project = await this.parseProject(projectPath);
        if (!project) continue;

        projects.push(project);
      } else {
        assertUnreachable(child);
      }
    }
  }

  /**
   * Recursivelly build path to the of the Group or FileRef. Relative path should be later resolved to full path
   * by joining it with xcworkspaceParentPath
   */
  buildLocation(item: XcodeWorksaceItem, ancestors: XcodeWorkspaceGroup[]): string | null {
    /* self: workspace is inside the project
     * container: relative to workspace dir
     * group: relative to group
     * developer: relative to developer dir
     * absolute: absolute path
     */
    const location = item.location;
    if (!location) {
      commonLogger.debug("Item has no location", { item: item });
      return null;
    }

    // No need to resolve path, it's already full path. This can happen with any object type
    if (path.isAbsolute(location.path)) {
      commonLogger.debug("Location is absolute", { location: location });

      return location.path;
    }

    // Relative path to the ".xcodeproj" directory that contains ".xcworkspace"
    if (location.obj === "self") {
      commonLogger.debug("Location is self", { location: location });
      return path.join("..", "..", location.path);
    }

    // Relative path to the parent directory of ".xcworkspace"
    if (location.obj === "container") {
      commonLogger.debug("Location is container", { location: location });
      return path.join(this.xcworkspaceParentPath, location.path);
    }

    // Relative path to the group
    if (location.obj === "group") {
      commonLogger.debug("Location is group", { location: location });
      const group = ancestors.at(-1);
      if (!group) {
        return location.path;
      }

      // Go deeper by resolving group path recursively, until we reach the root group
      const groupPath = this.buildLocation(group, ancestors.slice(0, -1));
      if (!groupPath) {
        return location.path;
      }
      return path.join(groupPath, location.path);
    }

    // Relative path to the developer directory (not sure what it is and how to resolve it)
    if (location.obj === "developer") {
      commonLogger.debug("Location is developer", { location: location });
      return null;
    }

    // Absolute path
    // We already check it at the beginning, but it's here for completness and sometimes it can have relative path
    // Ex: "absolute:../std/test1.hpp"
    if (location.obj === "absolute") {
      commonLogger.debug("Location is absolute (2)", { location: location });
      if (path.isAbsolute(location.path)) {
        return location.path;
      }
      return path.join(this.xcworkspaceParentPath, location.path);
    }
    commonLogger.debug("Unknown location type", { location: location });
    assertUnreachable(location.obj);
  }

  /**
   * Check if FileRef is referencing to Xcode project (".xcodeproj" directory) and return it's path
   *
   * Additional info:
   *  - https://github.com/microsoft/WinObjC/blob/master/tools/vsimporter/src/PBX/XCWorkspace.cpp#L114
   */
  async resolveProjectPath(ancestors: XcodeWorkspaceGroup[], fileRef: XcodeWorkspaceFileRef): Promise<string | null> {
    // We are looking for ".xcodeproj" files only (It's Xcode project file)
    const projectExtension = ".xcodeproj";

    // Ex: "./MyApp/TestWorkspace.xcworkspace" -> "./MyApp"
    const xcworkspaceParent = this.xcworkspaceParentPath;

    // In most basic form ".xcodeproj" directory contains "project.workspace" directory. For example:
    // "./Test1/test1.xcodeproj/project.workspace". In this case we can just check if parent directory of the
    // workspace is ".xcodeproj" directory and return it.
    //
    // Example:
    //   MyApp.xcodeproj
    //   ├── project.pbxproj
    //   ├── project.xcworkspace
    //   |   └── contents.xcworkspacedata
    //   └── ...
    if (xcworkspaceParent.endsWith(projectExtension)) {
      commonLogger.debug("Parent of xcworkspace is xcodeproj", { xcworkspaceParent: xcworkspaceParent });
      return xcworkspaceParent;
    }

    const filRefPath = fileRef.location.path;

    // Path is already absolute and ends with ".xcodeproj"
    // Ex: "/Users/user/MyApp/Test1/test1.xcodeproj"
    if (path.isAbsolute(filRefPath)) {
      commonLogger.debug("FileRef path is absolute", { filRefPath: filRefPath });
      if (filRefPath.endsWith(projectExtension)) {
        return filRefPath;
      }
      return null;
    }

    // Recursivelly build path of the FileRef
    let resolvedPath = this.buildLocation(fileRef, ancestors);
    if (!resolvedPath) {
      commonLogger.debug("No resolved path", { fileRef: fileRef });
      return null;
    }

    commonLogger.debug("Resolved path", { resolvedPath: resolvedPath });

    // Make it absolute path by joining the parent directory of ".xcworkspace"
    if (!path.isAbsolute(resolvedPath)) {
      resolvedPath = path.normalize(path.join(this.xcworkspaceParentPath, resolvedPath));
    }
    if (resolvedPath.endsWith(projectExtension)) {
      return resolvedPath;
    }

    return null;
  }

  /**
   * Parse <FileRef /> element from XML
   *
   * Example:
   *  - <FileRef location="group:Test1/test1.xcodeproj" />
   *  - <FileRef location="container:" />
   */
  static parseWorkspaceFileRef(options: { element: XmlElement }): XcodeWorkspaceFileRef | null {
    // Example:
    //  - "group:Test1/test1.xcodeproj"
    //  - "container:"
    //  - "group:../std/test1.hpp"
    const locationRaw: string | undefined = options.element.attributes.location;

    // We skip FileRef without location, because it's not clear what it's referencing to
    if (!locationRaw) {
      return null;
    }

    const location = parseLocation(locationRaw);

    return new XcodeWorkspaceFileRef({
      location,
    });
  }

  /**
   * Parse <Group /> element from XML
   *
   * Example:
   *  <Group location="group:Test1/test1.xcodeproj" name="Test1">
   *    <FileRef location="group:Test1/SomeFile.swift" />
   *    <FileRef location="group:Test1/SomeFile2.swift" />
   *    <Group location="group:Test1/SomeGroup" name="SomeGroup">
   *      <FileRef location="group:Test1/SomeGroup/SomeFile.swift" />
   *    </Group>
   *  </Group>
   */
  static parseWorkspaceItemGroup(options: { element: XmlElement }): XcodeWorkspaceGroup | null {
    const { element } = options;

    // Parse children, which can be either FileRef or Group
    const items: XcodeWorksaceItem[] = element.children
      .filter(isXMLElement)
      .map((obj) => XcodeWorkspace.parseWorkspaceItem({ element: obj }))
      .filter(isNotNull);

    // Example:
    //  - "group:Test1"
    //  - "group:../std"
    const locationRaw: string | undefined = element.attributes.location;

    const location = locationRaw ? parseLocation(locationRaw) : undefined;

    return new XcodeWorkspaceGroup({
      name: element.attributes.name,
      location: location,
      children: items,
    });
  }

  /**
   * Recursivelly parse either <FileRef /> or <Group /> element from XML
   */
  static parseWorkspaceItem(options: { element: XmlElement }): XcodeWorksaceItem | null {
    const { element } = options;

    // FileRef contains reference to xcodeproject or other files
    if (element.name === "FileRef") {
      return XcodeWorkspace.parseWorkspaceFileRef(options);
    }

    // Groups can contain other groups or FileRefs
    if (element.name === "Group") {
      return XcodeWorkspace.parseWorkspaceItemGroup(options);
    }
    return null;
  }

  /**
   * Given path to ".xcworkspace" directory, parse it and return parsed workspace structure
   */
  static async parseWorkspace(xcworkspacePath: string): Promise<XcodeWorkspace> {
    // Xcode store workspace structure in *.xcworkspace/contents.xcworkspacedata
    // It's XML fil with structure like:
    // <Workspace
    //   version = "1.0">
    //   <FileRef
    //     location = "group:TestContainer.xcodeproj">
    //   </FileRef>
    //   <Group
    //     location = "group:Test1/test1.xcodeproj"
    //     name = "Test1">
    //     <FileRef
    //       location = "group:Test1/SomeFile.swift">
    //     </FileRef>
    //   </Group>
    // </Workspace>
    // Read contents file

    // Currently only "contents.xcworkspacedata" is needed to parse workspace
    const contentsPath = path.join(xcworkspacePath, "contents.xcworkspacedata");
    return XcodeWorkspace.parseContentsWorkspaceData(contentsPath);
  }

  /**
   * Parse "contents.xcworkspacedata" file from ".xcworkspace" directory
   * and return parsed workspace structure
   */
  static async parseContentsWorkspaceData(contentsPath: string): Promise<XcodeWorkspace> {
    commonLogger.debug("Parsing contents.xcworkspacedata", { contentsPath: contentsPath });

    // Parent directory of contents.xcworkspacedata is .xcworkspace directory
    const xcworkspacePath = path.dirname(contentsPath);

    // contents.xcworkspacedata is just XML file wich contains workspace structure
    // of FileRefs and Groups
    const contentsData = await readFile(contentsPath);
    const contentsString = contentsData.toString();
    const contentsParsed = parseXml(contentsString);
    const contentsRoot = contentsParsed.root;
    if (!contentsRoot) {
      throw Error(`No root node found in ${contentsPath}`);
    }

    // Parse children of root node (FileRefs and Groups)
    const children = contentsRoot.children
      .filter(isXMLElement)
      .map((item) => XcodeWorkspace.parseWorkspaceItem({ element: item }))
      .filter(isNotNull);

    return new XcodeWorkspace({
      xcworkspacePath: xcworkspacePath,
      children: children,
    });
  }
}
