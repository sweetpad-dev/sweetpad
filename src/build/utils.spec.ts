import { XcodeCommandBuilder, detectWorkspaceType, getSwiftPMDirectory } from "./utils";

describe("XcodeCommandBuilder", () => {
  it("builds basic command with action", () => {
    const builder = new XcodeCommandBuilder();
    builder.addAction("build");
    const result = builder.build();
    expect(result[result.length - 1]).toBe("build");
  });

  it("adds parameters", () => {
    const builder = new XcodeCommandBuilder();
    builder.addParameters("-scheme", "MyApp");
    builder.addAction("build");
    const result = builder.build();
    expect(result).toContain("-scheme");
    expect(result).toContain("MyApp");
    expect(result).toContain("build");
  });

  it("adds build settings", () => {
    const builder = new XcodeCommandBuilder();
    builder.addBuildSettings("ONLY_ACTIVE_ARCH", "YES");
    const result = builder.build();
    expect(result).toContain("ONLY_ACTIVE_ARCH=YES");
  });

  it("adds options (flags without values)", () => {
    const builder = new XcodeCommandBuilder();
    builder.addOption("-allowProvisioningUpdates");
    const result = builder.build();
    expect(result).toContain("-allowProvisioningUpdates");
  });

  it("puts build settings before parameters before actions", () => {
    const builder = new XcodeCommandBuilder();
    builder.addAction("build");
    builder.addParameters("-scheme", "MyApp");
    builder.addBuildSettings("CODE_SIGN_IDENTITY", "-");

    const result = builder.build();
    const settingIdx = result.indexOf("CODE_SIGN_IDENTITY=-");
    const schemeIdx = result.indexOf("-scheme");
    const buildIdx = result.indexOf("build");

    expect(settingIdx).toBeLessThan(schemeIdx);
    expect(schemeIdx).toBeLessThan(buildIdx);
  });

  describe("addAdditionalArgs", () => {
    it("parses -arg value pairs", () => {
      const builder = new XcodeCommandBuilder();
      builder.addAdditionalArgs(["-arg1", "value1", "-arg2", "value2"]);
      const result = builder.build();
      expect(result).toContain("-arg1");
      expect(result).toContain("value1");
      expect(result).toContain("-arg2");
      expect(result).toContain("value2");
    });

    it("parses standalone flags", () => {
      const builder = new XcodeCommandBuilder();
      builder.addAdditionalArgs(["-flag1", "-flag2"]);
      const result = builder.build();
      expect(result).toContain("-flag1");
      expect(result).toContain("-flag2");
    });

    it("parses KEY=value build settings", () => {
      const builder = new XcodeCommandBuilder();
      builder.addAdditionalArgs(["ARG1=value1", "ARG2=value2"]);
      const result = builder.build();
      expect(result).toContain("ARG1=value1");
      expect(result).toContain("ARG2=value2");
    });

    it("parses actions like clean, build, test", () => {
      const builder = new XcodeCommandBuilder();
      builder.addAdditionalArgs(["clean", "build"]);
      const result = builder.build();
      expect(result).toContain("clean");
      expect(result).toContain("build");
    });

    it("deduplicates parameters keeping last occurrence", () => {
      const builder = new XcodeCommandBuilder();
      builder.addParameters("-scheme", "Original");
      builder.addAdditionalArgs(["-scheme", "Override"]);
      const result = builder.build();
      const schemeValues = result.filter((_, i) => result[i - 1] === "-scheme");
      expect(schemeValues).toEqual(["Override"]);
    });

    it("deduplicates actions", () => {
      const builder = new XcodeCommandBuilder();
      builder.addAction("build");
      builder.addAdditionalArgs(["build"]);
      const result = builder.build();
      const buildCount = result.filter((v) => v === "build").length;
      expect(buildCount).toBe(1);
    });

    it("deduplicates build settings keeping last occurrence", () => {
      const builder = new XcodeCommandBuilder();
      builder.addBuildSettings("KEY", "old");
      builder.addAdditionalArgs(["KEY=new"]);
      const result = builder.build();
      expect(result).toContain("KEY=new");
      expect(result).not.toContain("KEY=old");
    });

    it("handles empty args", () => {
      const builder = new XcodeCommandBuilder();
      builder.addAdditionalArgs([]);
      const result = builder.build();
      // Should just have the xcodebuild command
      expect(result).toHaveLength(1);
    });
  });
});

describe("detectWorkspaceType", () => {
  it("returns 'spm' for Package.swift", () => {
    expect(detectWorkspaceType("/path/to/Package.swift")).toBe("spm");
  });

  it("returns 'xcode' for .xcworkspace", () => {
    expect(detectWorkspaceType("/path/to/MyApp.xcworkspace")).toBe("xcode");
  });

  it("returns 'xcode' for .xcodeproj workspace", () => {
    expect(detectWorkspaceType("/path/to/MyApp.xcodeproj/project.xcworkspace")).toBe("xcode");
  });
});

describe("getSwiftPMDirectory", () => {
  it("returns parent directory of Package.swift", () => {
    expect(getSwiftPMDirectory("/Users/dev/MyPackage/Package.swift")).toBe("/Users/dev/MyPackage");
  });

  it("throws for non-SPM paths", () => {
    expect(() => getSwiftPMDirectory("/path/to/MyApp.xcworkspace")).toThrow();
  });
});
