import path from "node:path";
import type { XcodeProject } from "./project";
import { XcodeWorkspace, XcodeWorkspaceFileRef, XcodeWorkspaceGroup } from "./workspace";

describe("parse *.xcworkspace/contents.xml", () => {
  const DATA_DIR = path.join(process.cwd(), "tests", "contents-data");
  const TESTS_DIR = path.join(process.cwd(), "tests");

  it("should parse contents1.xml", async () => {
    const workspace = await XcodeWorkspace.parseContentsWorkspaceData(path.join(DATA_DIR, "content1.xml"));
    expect(workspace).toBeDefined();
    expect(workspace.children).toHaveLength(2);

    const item1 = workspace.children[0] as XcodeWorkspaceFileRef;
    expect(item1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item1.location.obj).toBe("self");
    expect(item1.location.path).toBe("Test1.xcodeproj");

    const item2 = workspace.children[1] as XcodeWorkspaceFileRef;
    expect(item2).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item2.location.obj).toBe("self");
    expect(item2.location.path).toBe("");
  });

  it("should parse contents2.xml", async () => {
    const workspace = await XcodeWorkspace.parseContentsWorkspaceData(path.join(DATA_DIR, "content2.xml"));
    expect(workspace).toBeDefined();
    expect(workspace.children).toHaveLength(2);
    const item1 = workspace.children[0] as XcodeWorkspaceFileRef;
    expect(item1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item1.location.obj).toBe("group");
    expect(item1.location.path).toBe("test1.xcodeproj");

    const item2 = workspace.children[1] as XcodeWorkspaceFileRef;
    expect(item2).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item2.location.obj).toBe("group");
    expect(item2.location.path).toBe("Pods/Pods.xcodeproj");
  });

  it("should parse contents3.xml", async () => {
    const workspace = await XcodeWorkspace.parseContentsWorkspaceData(path.join(DATA_DIR, "content3.xml"));
    expect(workspace).toBeDefined();
    expect(workspace.children).toHaveLength(3);

    const item1 = workspace.children[0] as XcodeWorkspaceFileRef;
    expect(item1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item1.location.obj).toBe("container");
    expect(item1.location.path).toBe("TestContainer.xcodeproj");

    const item2 = workspace.children[1] as XcodeWorkspaceFileRef;
    expect(item2).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item2.location.obj).toBe("group");
    expect(item2.location.path).toBe("Test1/test1.xcodeproj");

    const item3 = workspace.children[2] as XcodeWorkspaceFileRef;
    expect(item3).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item3.location.obj).toBe("group");
    expect(item3.location.path).toBe("Test2/test2.xcodeproj");
  });

  it("should parse contents4.xml", async () => {
    const workspace = await XcodeWorkspace.parseContentsWorkspaceData(path.join(DATA_DIR, "content4.xml"));
    expect(workspace).toBeDefined();
    expect(workspace.children).toHaveLength(5);

    const item1 = workspace.children[0] as XcodeWorkspaceFileRef;
    expect(item1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item1.location.obj).toBe("container");
    expect(item1.location.path).toBe("test.xcconfig");

    const item2 = workspace.children[1] as XcodeWorkspaceGroup;
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

    const item3 = workspace.children[2] as XcodeWorkspaceGroup;
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

    const item4 = workspace.children[3] as XcodeWorkspaceFileRef;
    expect(item4).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item4.location.obj).toBe("container");
    expect(item4.location.path).toBe("ge0/ge0.xcodeproj");

    const item5 = workspace.children[4] as XcodeWorkspaceFileRef;
    expect(item5).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item5.location.obj).toBe("group");
    expect(item5.location.path).toBe("../iphone/CoreApi/CoreApi.xcodeproj");
  });

  it("should parse contents5.xml", async () => {
    const workspace = await XcodeWorkspace.parseContentsWorkspaceData(path.join(DATA_DIR, "content5.xml"));
    expect(workspace).toBeDefined();
    expect(workspace.children).toHaveLength(1);

    const item1 = workspace.children[0] as XcodeWorkspaceFileRef;
    expect(item1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item1.location.obj).toBe("self");
    expect(item1.location.path).toBe("/Users/mwnl/Desktop/iOSApplicationOutside 2/iOSAppMobile.xcodeproj");
  });

  it("should parse contents6.xml", async () => {
    const workspace = await XcodeWorkspace.parseContentsWorkspaceData(path.join(DATA_DIR, "content6.xml"));
    expect(workspace).toBeDefined();
    expect(workspace.children).toHaveLength(1);

    const item1 = workspace.children[0] as XcodeWorkspaceGroup;
    expect(item1).toBeInstanceOf(XcodeWorkspaceGroup);
    expect(item1.children).toHaveLength(2);
    expect(item1.location?.obj).toBe("group");
    expect(item1.location?.path).toBe("Projects");

    const item1_child1 = item1.children[0] as XcodeWorkspaceFileRef;
    expect(item1_child1).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item1_child1.location.obj).toBe("group");
    expect(item1_child1.location.path).toBe("AFramework/AFramework.xcodeproj");

    const item1_child2 = item1.children[1] as XcodeWorkspaceFileRef;
    expect(item1_child2).toBeInstanceOf(XcodeWorkspaceFileRef);
    expect(item1_child2.location.obj).toBe("group");
    expect(item1_child2.location.path).toBe("App/sweetpad-test.xcodeproj");
  });
});

describe("get projects *.xcworkspace/contents.xcworkspacedata", () => {
  const DATA_DIR = path.join(process.cwd(), "tests", "contents-data");

  async function testCase(options: {
    contentsPath: string;
    mockXcworkspacePath: string;
    expectedPaths: string[];
  }) {
    const workspace = await XcodeWorkspace.parseContentsWorkspaceData(path.join(DATA_DIR, options.contentsPath));
    workspace.xcworkspacePath = options.mockXcworkspacePath;

    jest.spyOn(workspace, "parseProject").mockImplementation(async (path: string) => {
      return {
        projectPath: path,
      } as any;
    });
    const projects = await workspace.getProjects();
    const projectsPaths = projects.map((project) => (project as XcodeProject).projectPath);
    expect(projectsPaths).toHaveLength(options.expectedPaths.length);
    expect(projectsPaths.sort()).toEqual(options.expectedPaths.sort());
  }

  it("should get projects from content1.xml", async () => {
    await testCase({
      contentsPath: "content1.xml",
      mockXcworkspacePath: "/Users/someone/Developer/MyApp/Test1.xcodeproj/project.xcworkspace",
      expectedPaths: ["/Users/someone/Developer/MyApp/Test1.xcodeproj"],
    });
  });

  it("should get projects from content2.xml", async () => {
    await testCase({
      contentsPath: "content2.xml",
      mockXcworkspacePath: "/Users/someone/Developer/MyApp/test1.xcworkspace",
      expectedPaths: [
        "/Users/someone/Developer/MyApp/test1.xcodeproj",
        "/Users/someone/Developer/MyApp/Pods/Pods.xcodeproj",
      ],
    });
  });

  it("should get projects from content3.xml", async () => {
    await testCase({
      contentsPath: "content3.xml",
      mockXcworkspacePath: "/Users/someone/Developer/MyApp/TestContainer.xcworkspace",
      expectedPaths: [
        "/Users/someone/Developer/MyApp/TestContainer.xcodeproj",
        "/Users/someone/Developer/MyApp/Test1/test1.xcodeproj",
        "/Users/someone/Developer/MyApp/Test2/test2.xcodeproj",
      ],
    });
  });

  it("should get projects from content4.xml", async () => {
    await testCase({
      contentsPath: "content4.xml",
      mockXcworkspacePath: "/Users/someone/Developer/MyApp/xcode/MyTestApp.xcworkspace",
      expectedPaths: [
        "/Users/someone/Developer/MyApp/iphone/CoreApi/CoreApi.xcodeproj",
        "/Users/someone/Developer/MyApp/xcode/ge0/ge0.xcodeproj",
        "/Users/someone/Developer/MyApp/xcode/agg/agg.xcodeproj",
        "/Users/someone/Developer/MyApp/xcode/alohalitics/alohalitics.xcodeproj",
      ],
    });
  });

  it("should get projects from content5.xml", async () => {
    await testCase({
      contentsPath: "content5.xml",
      mockXcworkspacePath: "/Users/mwnl/Desktop/iOSApplicationOutside 2/iOSAppMobile.xcworkspace",
      expectedPaths: ["/Users/mwnl/Desktop/iOSApplicationOutside 2/iOSAppMobile.xcodeproj"],
    });
  });

  it("should get projects from content6.xml", async () => {
    await testCase({
      contentsPath: "content6.xml",
      mockXcworkspacePath: "/Users/someone/Developer/MyApp/sweetpad-test.xcworkspace",
      expectedPaths: [
        "/Users/someone/Developer/MyApp/Projects/AFramework/AFramework.xcodeproj",
        "/Users/someone/Developer/MyApp/Projects/App/sweetpad-test.xcodeproj",
      ],
    });
  });
});

describe("parse full projects", () => {
  // tests/examples/example-1/sweetpad-test.xcworkspace/contents.xcworkspacedata
  const TESTS_DIR = path.join(process.cwd(), "tests");
  const EXAMPLES_DIR = path.join(TESTS_DIR, "examples");

  it("it should parse metheor-ios.xcworkspace", async () => {
    const PROJECT_PATH = path.join(EXAMPLES_DIR, "meteor-ios");
    const workspace = await XcodeWorkspace.parseWorkspace(path.join(PROJECT_PATH, "Meteor.xcworkspace"));
    const projects = await workspace.getProjects();
    expect(projects).toHaveLength(3);

    const project1 = projects[0] as XcodeProject;
    expect(project1.projectPath).toBe(path.join(PROJECT_PATH, "Meteor.xcodeproj"));

    const project2 = projects[1] as XcodeProject;
    expect(project2.projectPath).toBe(path.join(PROJECT_PATH, "Examples", "Todos", "Todos.xcodeproj"));

    const project3 = projects[2] as XcodeProject;
    expect(project3.projectPath).toBe(path.join(PROJECT_PATH, "Examples", "Leaderboard", "Leaderboard.xcodeproj"));
  });

  it("it should parse sweetpad-demo-cocoapods.xcworkspace", async () => {
    const PROJECT_PATH = path.join(EXAMPLES_DIR, "sweetpad-demo-cocoapods");
    const workspace = await XcodeWorkspace.parseWorkspace(
      path.join(PROJECT_PATH, "sweetpad-demo-cocoapods.xcworkspace"),
    );
    const projects = await workspace.getProjects();
    expect(projects).toHaveLength(2);

    const project1 = projects[0] as XcodeProject;
    expect(project1.projectPath).toBe(path.join(PROJECT_PATH, "sweetpad-demo-cocoapods.xcodeproj"));

    const project2 = projects[1] as XcodeProject;
    expect(project2.projectPath).toBe(path.join(PROJECT_PATH, "Pods", "Pods.xcodeproj"));
  });

  it("it should parse sweetpad-demo-xcodegen.xcodeproj", async () => {
    const PROJECT_PATH = path.join(EXAMPLES_DIR, "sweetpad-demo-xcodegen");
    const workspace = await XcodeWorkspace.parseWorkspace(
      path.join(PROJECT_PATH, "sweetpad-demo-xcodegen.xcodeproj", "project.xcworkspace"),
    );
    const projects = await workspace.getProjects();
    expect(projects).toHaveLength(1);

    const project1 = projects[0] as XcodeProject;
    expect(project1.projectPath).toBe(path.join(PROJECT_PATH, "sweetpad-demo-xcodegen.xcodeproj"));
  });

  it("it should parse terminal23.xcworkspace", async () => {
    const PROJECT_PATH = path.join(EXAMPLES_DIR, "terminal23");
    const workspace = await XcodeWorkspace.parseWorkspace(
      path.join(PROJECT_PATH, "terminal23.xcodeproj", "project.xcworkspace"),
    );
    const projects = await workspace.getProjects();
    expect(projects).toHaveLength(1);

    const project1 = projects[0] as XcodeProject;
    expect(project1.projectPath).toBe(path.join(PROJECT_PATH, "terminal23.xcodeproj"));
  });

  it("it should parse sweetpad-test.xcworkspace", async () => {
    const PROJECT_PATH = path.join(EXAMPLES_DIR, "sweetpad-multiproject");
    const workspace = await XcodeWorkspace.parseWorkspace(path.join(PROJECT_PATH, "sweetpad-test.xcworkspace"));
    const projects = await workspace.getProjects();
    expect(projects).toHaveLength(2);

    const project1 = projects[0] as XcodeProject;
    expect(project1.projectPath).toBe(path.join(PROJECT_PATH, "Projects", "AFramework", "AFramework.xcodeproj"));

    const project2 = projects[1] as XcodeProject;
    expect(project2.projectPath).toBe(path.join(PROJECT_PATH, "Projects", "App", "sweetpad-test.xcodeproj"));
  });

  it("it should parse Ampol.xcworkspace", async () => {
    const PROJECT_PATH = path.join(EXAMPLES_DIR, "Ampol");
    const workspace = await XcodeWorkspace.parseWorkspace(path.join(PROJECT_PATH, "Ampol.xcworkspace"));
    const projects = await workspace.getProjects();
    expect(projects).toHaveLength(1);

    const project1 = projects[0] as XcodeProject;
    expect(project1.projectPath).toBe(path.join(PROJECT_PATH, "Ampol.xcodeproj"));
  });

  it("it should parse Runner.xcworkspace", async () => {
    const PROJECT_PATH = path.join(EXAMPLES_DIR, "take_notes");
    const workspace = await XcodeWorkspace.parseWorkspace(path.join(PROJECT_PATH, "ios", "Runner.xcworkspace"));
    const projects = await workspace.getProjects();
    expect(projects).toHaveLength(1);

    const project1 = projects[0] as XcodeProject;
    expect(project1.projectPath).toBe(path.join(PROJECT_PATH, "ios", "Runner.xcodeproj"));
  });
});
