import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import {
  dylibNameFor,
  findPackageResolvedFiles,
  pinsContainInject,
  platformDirNameFor,
  prependPath,
  sdkSupportsHotReload,
} from "./hot-reload";

describe("dylibNameFor", () => {
  it("maps each supported destination to its lib*Injection.dylib filename", () => {
    expect(dylibNameFor("iOSSimulator")).toBe("libiphonesimulatorInjection.dylib");
    expect(dylibNameFor("visionOSSimulator")).toBe("libxrsimulatorInjection.dylib");
    expect(dylibNameFor("tvOSSimulator")).toBe("libappletvsimulatorInjection.dylib");
    expect(dylibNameFor("macOS")).toBe("libmacosxInjection.dylib");
  });

  it("returns null for watchOS and every physical device (no injection dylib ships)", () => {
    expect(dylibNameFor("watchOSSimulator")).toBeNull();
    expect(dylibNameFor("iOSDevice")).toBeNull();
    expect(dylibNameFor("tvOSDevice")).toBeNull();
    expect(dylibNameFor("watchOSDevice")).toBeNull();
    expect(dylibNameFor("visionOSDevice")).toBeNull();
  });
});

describe("platformDirNameFor", () => {
  it("maps each supported destination to its Xcode <Platform>.platform dir name", () => {
    expect(platformDirNameFor("iOSSimulator")).toBe("iPhoneSimulator");
    expect(platformDirNameFor("visionOSSimulator")).toBe("XRSimulator");
    expect(platformDirNameFor("tvOSSimulator")).toBe("AppleTVSimulator");
    expect(platformDirNameFor("macOS")).toBe("MacOSX");
  });

  it("returns null for destinations we don't compute XCTest search paths for", () => {
    expect(platformDirNameFor("watchOSSimulator")).toBeNull();
    expect(platformDirNameFor("iOSDevice")).toBeNull();
    expect(platformDirNameFor("tvOSDevice")).toBeNull();
    expect(platformDirNameFor("watchOSDevice")).toBeNull();
    expect(platformDirNameFor("visionOSDevice")).toBeNull();
  });
});

describe("sdkSupportsHotReload", () => {
  it.each(["iphonesimulator", "appletvsimulator", "xrsimulator", "macosx"])("accepts %s", (sdk) => {
    expect(sdkSupportsHotReload(sdk)).toBe(true);
  });

  it.each(["iphoneos", "appletvos", "xros", "watchos", "watchsimulator", ""])("rejects %s", (sdk) => {
    expect(sdkSupportsHotReload(sdk)).toBe(false);
  });
});

describe("prependPath", () => {
  it("returns the value alone when no existing path is set", () => {
    expect(prependPath(undefined, "/a")).toBe("/a");
  });

  it("returns the value alone when the existing path is empty", () => {
    expect(prependPath("", "/a")).toBe("/a");
  });

  it("prepends with a colon separator and preserves the existing order", () => {
    expect(prependPath("/orig", "/new")).toBe("/new:/orig");
    expect(prependPath("/x:/y", "/z")).toBe("/z:/x:/y");
  });
});

describe("pinsContainInject", () => {
  it("returns false for non-object input", () => {
    expect(pinsContainInject(null)).toBe(false);
    expect(pinsContainInject(undefined)).toBe(false);
    expect(pinsContainInject(42)).toBe(false);
    expect(pinsContainInject("string")).toBe(false);
  });

  it("returns false when pins is missing or not an array", () => {
    expect(pinsContainInject({})).toBe(false);
    expect(pinsContainInject({ pins: null })).toBe(false);
    expect(pinsContainInject({ pins: "nope" })).toBe(false);
  });

  it("returns false when no pin matches the Inject repo", () => {
    expect(
      pinsContainInject({
        pins: [
          { identity: "swift-foo", location: "https://github.com/apple/swift-foo.git" },
          { location: "https://github.com/other/Inject-Like" },
        ],
      }),
    ).toBe(false);
  });

  it("matches the canonical repo URL with and without the .git suffix", () => {
    expect(
      pinsContainInject({
        pins: [{ location: "https://github.com/krzysztofzablocki/Inject" }],
      }),
    ).toBe(true);

    expect(
      pinsContainInject({
        pins: [{ location: "https://github.com/krzysztofzablocki/Inject.git" }],
      }),
    ).toBe(true);
  });

  it("is case-insensitive on the owner/repo segment", () => {
    expect(
      pinsContainInject({
        pins: [{ location: "https://github.com/KrzysztofZablocki/INJECT.git" }],
      }),
    ).toBe(true);
  });

  it("skips malformed pin entries and still finds a valid match later in the array", () => {
    expect(
      pinsContainInject({
        pins: [null, "string", { location: 42 }, { location: "https://github.com/krzysztofzablocki/Inject.git" }],
      }),
    ).toBe(true);
  });
});

describe("findPackageResolvedFiles", () => {
  let tmpDir: string;

  beforeEach(async () => {
    tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-hotreload-"));
  });

  afterEach(async () => {
    await fs.rm(tmpDir, { recursive: true, force: true });
  });

  it("returns an empty list when the workspace has no Package.resolved anywhere", async () => {
    expect(await findPackageResolvedFiles(tmpDir)).toEqual([]);
  });

  it("returns the Package.resolved at the workspace root", async () => {
    const root = path.join(tmpDir, "Package.resolved");
    await fs.writeFile(root, "{}");
    expect(await findPackageResolvedFiles(tmpDir)).toEqual([root]);
  });

  it("finds Package.resolved nested under a .xcworkspace", async () => {
    const dir = path.join(tmpDir, "App.xcworkspace", "xcshareddata", "swiftpm");
    await fs.mkdir(dir, { recursive: true });
    const file = path.join(dir, "Package.resolved");
    await fs.writeFile(file, "{}");
    expect(await findPackageResolvedFiles(tmpDir)).toEqual([file]);
  });

  it("finds Package.resolved nested under an .xcodeproj's project.xcworkspace", async () => {
    const dir = path.join(tmpDir, "App.xcodeproj", "project.xcworkspace", "xcshareddata", "swiftpm");
    await fs.mkdir(dir, { recursive: true });
    const file = path.join(dir, "Package.resolved");
    await fs.writeFile(file, "{}");
    expect(await findPackageResolvedFiles(tmpDir)).toEqual([file]);
  });

  it("returns every Package.resolved it finds when several locations exist", async () => {
    const root = path.join(tmpDir, "Package.resolved");
    await fs.writeFile(root, "{}");

    const wsDir = path.join(tmpDir, "App.xcworkspace", "xcshareddata", "swiftpm");
    await fs.mkdir(wsDir, { recursive: true });
    const wsFile = path.join(wsDir, "Package.resolved");
    await fs.writeFile(wsFile, "{}");

    expect((await findPackageResolvedFiles(tmpDir)).toSorted()).toEqual([root, wsFile].toSorted());
  });

  it("returns an empty list when the workspace directory does not exist", async () => {
    expect(await findPackageResolvedFiles(path.join(tmpDir, "missing"))).toEqual([]);
  });

  it("ignores .xcworkspace and .xcodeproj entries that don't actually contain Package.resolved", async () => {
    await fs.mkdir(path.join(tmpDir, "Empty.xcworkspace"));
    await fs.mkdir(path.join(tmpDir, "Empty.xcodeproj"));
    expect(await findPackageResolvedFiles(tmpDir)).toEqual([]);
  });
});
