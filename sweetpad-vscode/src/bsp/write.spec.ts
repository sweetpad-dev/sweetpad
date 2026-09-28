import { assembleBspConfig } from "./write";

vi.mock("../common/config", () => ({ getWorkspaceConfig: vi.fn(() => undefined) }));

// The extension's builds add `sweetpad.build.args` to the xcodebuild command
// line. The BSP server reads them from bsp.json, so the index resolves each
// target the way those builds do.
describe("assembleBspConfig", () => {
  it("carries the build args into the config as given", () => {
    const buildArgs = ["-xcconfig", "ci.xcconfig", "CODE_SIGNING_ALLOWED=NO"];
    const config = assembleBspConfig({
      workspacePath: "/w",
      xcworkspace: "/w/App.xcodeproj/project.xcworkspace",
      developerDir: null,
      scheme: "App",
      configuration: "Debug",
      destinationPlatform: null,
      derivedDataPath: null,
      buildArgs: buildArgs,
    });
    expect(config.projectPath).toBe("/w/App.xcodeproj");
    expect(config.buildArgs).toEqual(buildArgs);
  });
});
