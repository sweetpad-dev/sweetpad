import path from "path";
import { XcodeWorkspace, XcodeWorkspaceFileRef, XcodeWorkspaceGroup } from "./workspace";

describe("parse *.xcworkspace/contents.xml", () => {
  const DATA_DIR = path.join(process.cwd(), "tests", "contents-data");
  const PARENT_DIR = path.dirname(DATA_DIR);

  it("should parse contents1.xml", async () => {
    const workspace = await XcodeWorkspace.parseContents(path.join(DATA_DIR, "content1.xml"));
    expect(workspace).toBeDefined();
    expect(workspace.items).toHaveLength(2);

    const item1 = workspace.items[0] as XcodeWorkspaceFileRef;
    expect(item1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item1.location.obj).toBe("self");
    expect(item1.location.path).toBe("Test1.xcodeproj");

    const item2 = workspace.items[1] as XcodeWorkspaceFileRef;
    expect(item2).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item2.location.obj).toBe("self");
    expect(item2.location.path).toBe("");
  });

  it("should parse contents2.xml", async () => {
    const workspace = await XcodeWorkspace.parseContents(path.join(DATA_DIR, "content2.xml"));
    expect(workspace).toBeDefined();
    expect(workspace.items).toHaveLength(2);
    const item1 = workspace.items[0] as XcodeWorkspaceFileRef;
    expect(item1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item1.location.obj).toBe("group");
    expect(item1.location.path).toBe("test1.xcodeproj");

    const item2 = workspace.items[1] as XcodeWorkspaceFileRef;
    expect(item2).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item2.location.obj).toBe("group");
    expect(item2.location.path).toBe("Pods/Pods.xcodeproj");
  });

  it("should parse contents3.xml", async () => {
    const workspace = await XcodeWorkspace.parseContents(path.join(DATA_DIR, "content3.xml"));
    expect(workspace).toBeDefined();
    expect(workspace.items).toHaveLength(3);

    const item1 = workspace.items[0] as XcodeWorkspaceFileRef;
    expect(item1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item1.location.obj).toBe("container");
    expect(item1.location.path).toBe("TestContainer.xcodeproj");

    const item2 = workspace.items[1] as XcodeWorkspaceFileRef;
    expect(item2).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item2.location.obj).toBe("group");
    expect(item2.location.path).toBe("Test1/test1.xcodeproj");

    const item3 = workspace.items[2] as XcodeWorkspaceFileRef;
    expect(item3).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item3.location.obj).toBe("group");
    expect(item3.location.path).toBe("Test2/test2.xcodeproj");
  });

  it("should parse contents4.xml", async () => {
    const workspace = await XcodeWorkspace.parseContents(path.join(DATA_DIR, "content4.xml"));
    expect(workspace).toBeDefined();
    expect(workspace.items).toHaveLength(5);

    const item1 = workspace.items[0] as XcodeWorkspaceFileRef;
    expect(item1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item1.location.obj).toBe("container");
    expect(item1.location.path).toBe("test.xcconfig");

    const item2 = workspace.items[1] as XcodeWorkspaceGroup;
    expect(item2).toBeInstanceOf(XcodeWorkspaceGroup);
    expect(item2.name).toBe("std");
    expect(item2.location?.obj).toBe("group");
    expect(item2.location?.path).toBe("../std");

    const item2_child1 = item2.children[0] as XcodeWorkspaceFileRef;
    expect(item2_child1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item2_child1.location.obj).toBe("group");
    expect(item2_child1.location.path).toBe("../std/test1.hpp");

    const item2_child2 = item2.children[1] as XcodeWorkspaceFileRef;
    expect(item2_child2).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item2_child2.location.obj).toBe("group");
    expect(item2_child2.location.path).toBe("../std/test2.hpp");

    const item3 = workspace.items[2] as XcodeWorkspaceGroup;
    expect(item3).toBeInstanceOf(XcodeWorkspaceGroup);
    expect(item3.name).toBe("3party");
    expect(item3.location?.obj).toBe("container");
    expect(item3.location?.path).toBe("");

    const item3_child1 = item3.children[0] as XcodeWorkspaceFileRef;
    expect(item3_child1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item3_child1.location.obj).toBe("container");
    expect(item3_child1.location.path).toBe("agg/agg.xcodeproj");

    const item3_child2 = item3.children[1] as XcodeWorkspaceFileRef;
    expect(item3_child2).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item3_child2.location.obj).toBe("container");
    expect(item3_child2.location.path).toBe("alohalitics/alohalitics.xcodeproj");

    const item4 = workspace.items[3] as XcodeWorkspaceFileRef;
    expect(item4).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item4.location.obj).toBe("container");
    expect(item4.location.path).toBe("ge0/ge0.xcodeproj");

    const item5 = workspace.items[4] as XcodeWorkspaceFileRef;
    expect(item5).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item5.location.obj).toBe("group");
    expect(item5.location.path).toBe("../iphone/CoreApi/CoreApi.xcodeproj");
  });

  it("should parse contents5.xml", async () => {
    const workspace = await XcodeWorkspace.parseContents(path.join(DATA_DIR, "content5.xml"));
    expect(workspace).toBeDefined();
    expect(workspace.items).toHaveLength(1);

    const item1 = workspace.items[0] as XcodeWorkspaceFileRef;
    expect(item1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item1.location.obj).toBe("self");
    expect(item1.location.path).toBe("/Users/mwnl/Desktop/iOSApplicationOutside 2/iOSAppMobile.xcodeproj");
  });
});
