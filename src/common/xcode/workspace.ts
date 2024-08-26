import path from "node:path";
import { XmlElement, parseXml } from "@rgrove/parse-xml";
import { readFile } from "../files";
import { isNotNull } from "../types";
import { XcodeProject } from "./project";

/* self: workspace is inside the project
 * container: relative to workspace dir
 * group: relative to group
 * developer: relative to developer dir
 * absolute: absolute path
 */
const XCODE_WORKSPACE_LOCATIONS_TYPES = ["self", "container", "group", "developer", "absolute"] as const;
type XcodeWorkspaceLocationType = (typeof XCODE_WORKSPACE_LOCATIONS_TYPES)[number];

interface XcodeWorksaceItem {
  type: "group" | "fileref";
}

export class XcodeWorkspaceFileRef implements XcodeWorksaceItem {
  public readonly type = "fileref";
  public location: XcodeWorkspaceLocation;
  public workspacePath: string;

  constructor(options: { location: XcodeWorkspaceLocation; workspacePath: string }) {
    this.location = options.location;
    this.workspacePath = options.workspacePath;
  }

  getProjectPath(): string | null {
    const projectExtension = ".xcodeproj";
    const workspaceParent = path.dirname(this.workspacePath);
    const workspaceParentParent = path.dirname(workspaceParent);

    if (this.workspacePath.endsWith(projectExtension)) {
      return path.dirname(this.workspacePath);
    }

    // Somehone path is already contains project extension
    if (this.location.obj === "self") {
      if (workspaceParent.endsWith(projectExtension)) {
        return workspaceParent;
      }
      if (workspaceParentParent.endsWith(projectExtension)) {
        return workspaceParentParent;
      }
      if (path.isAbsolute(this.location.path) && this.location.path.endsWith(projectExtension)) {
        return this.location.path;
      }

      const selfCombined = path.join(workspaceParentParent, this.location.path);
      if (selfCombined.endsWith(projectExtension)) {
        return path.normalize(selfCombined);
      }
      return null;
    }

    // Path related to group
    if (this.location.obj === "group") {
      if (path.isAbsolute(this.location.path) && this.location.path.endsWith(projectExtension)) {
        return path.normalize(this.location.path);
      }
      const gropupCombined = path.join(workspaceParent, this.location.path);
      if (gropupCombined.endsWith(projectExtension)) {
        return path.normalize(gropupCombined);
      }
    }

    return null;
  }
}

function parseLocation(location: string): XcodeWorkspaceLocation {
  const [rawObj, path] = location.split(":", 2);
  const obj = rawObj as XcodeWorkspaceLocationType;

  return {
    obj: obj,
    path: path,
  };
}

export class XcodeWorkspaceGroup implements XcodeWorksaceItem {
  public readonly type = "group";
  public name: string | undefined;
  public location: XcodeWorkspaceLocation | undefined;
  public children: XcodeWorksaceItem[];
  public workspacePath: string;

  constructor(options: {
    name: string | undefined;
    location: XcodeWorkspaceLocation | undefined;
    children: XcodeWorksaceItem[];
    workspacePath: string;
  }) {
    this.name = options.name;
    this.location = options.location;
    this.children = options.children;
    this.workspacePath = options.workspacePath;
  }
}

type XcodeWorkspaceLocation = {
  obj: XcodeWorkspaceLocationType;
  path: string;
};

/**
 * WIP: I've added that class when I was on parsing xcode state from files,
 * but I didn't finish it
 */
export class XcodeWorkspace {
  // path to ".xcworkspace" (not "contents.xcworkspacedata")
  public path: string;
  public items: XcodeWorksaceItem[];

  get parentPath(): string {
    return path.dirname(this.path);
  }

  constructor(options: { path: string; items: XcodeWorksaceItem[] }) {
    this.path = options.path;
    this.items = options.items;
  }

  async getProjects(): Promise<XcodeProject[]> {
    let items: XcodeWorksaceItem[] = this.items;
    const projects: XcodeProject[] = [];
    while (items.length > 0) {
      const item = items.shift();
      if (item instanceof XcodeWorkspaceFileRef) {
        const project = await XcodeProject.fromFileRef(item);
        if (project) {
          projects.push(project);
        }
      } else if (item instanceof XcodeWorkspaceGroup) {
        items = items.concat(item.children);
      }
    }
    return projects;
  }

  static parseWorkspaceItem(options: { element: XmlElement; xcworkspace: string }): XcodeWorksaceItem | null {
    const { element } = options;

    // FileRef contains reference to xcodeproject or other files
    if (element.name === "FileRef") {
      const locationRaw: string | undefined = element.attributes.location;
      if (!locationRaw) {
        return null;
      }

      const location = parseLocation(locationRaw);

      return new XcodeWorkspaceFileRef({
        location: location,
        workspacePath: options.xcworkspace,
      });
    }

    // Groups can contain other groups or FileRefs
    if (element.name === "Group") {
      const items: XcodeWorksaceItem[] = element.children
        .map((obj) => {
          if (obj instanceof XmlElement) {
            return XcodeWorkspace.parseWorkspaceItem({
              element: obj,
              xcworkspace: options.xcworkspace,
            });
          }
          return null;
        })
        .filter(isNotNull);

      const locationRaw: string | undefined = element.attributes.location;

      const location = locationRaw ? parseLocation(locationRaw) : undefined;

      return new XcodeWorkspaceGroup({
        name: element.attributes.name,
        location: location,
        children: items,
        workspacePath: options.xcworkspace,
      });
    }
    return null;
  }

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
    const contentsPath = path.join(xcworkspacePath, "contents.xcworkspacedata");
    return XcodeWorkspace.parseContents(contentsPath);
  }

  static async parseContents(contentsPath: string): Promise<XcodeWorkspace> {
    const xcworkspace = path.dirname(contentsPath);

    const contentsData = await readFile(contentsPath);
    const contentsString = contentsData.toString();
    const contentsParsed = parseXml(contentsString);
    const contentsRoot = contentsParsed.root;
    if (!contentsRoot) {
      throw Error(`No root node found in ${contentsPath}`);
    }

    const items = contentsRoot.children
      .map((item) => {
        if (item instanceof XmlElement) {
          return XcodeWorkspace.parseWorkspaceItem({
            element: item,
            xcworkspace: xcworkspace,
          });
        }
        return null;
      })
      .filter(isNotNull);

    return new XcodeWorkspace({
      path: xcworkspace,
      items: items,
    });
  }
}
