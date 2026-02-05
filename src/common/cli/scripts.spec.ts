import * as vscode from "vscode";
import { ExtensionError } from "../errors";
import { getXcodeBuildCommand, parseCliJsonOutput } from "./scripts";

jest.mock("vscode");

const mockGetConfiguration = vscode.workspace.getConfiguration as jest.Mock;

describe("getXcodeBuildCommand", () => {
  const originalEnv = process.env;

  beforeEach(() => {
    jest.resetAllMocks();
    process.env = { ...originalEnv };
  });

  afterAll(() => {
    process.env = originalEnv;
  });

  it("returns default 'xcodebuild' when no config is set", () => {
    mockGetConfiguration.mockReturnValue({
      get: jest.fn().mockReturnValue(undefined),
    });
    expect(getXcodeBuildCommand()).toBe("xcodebuild");
  });

  it("returns default 'xcodebuild' when config is null", () => {
    mockGetConfiguration.mockReturnValue({
      get: jest.fn().mockReturnValue(null),
    });
    expect(getXcodeBuildCommand()).toBe("xcodebuild");
  });

  it("returns default 'xcodebuild' when config is empty string", () => {
    mockGetConfiguration.mockReturnValue({
      get: jest.fn().mockReturnValue(""),
    });
    expect(getXcodeBuildCommand()).toBe("xcodebuild");
  });

  it("returns custom command when configured with absolute path", () => {
    mockGetConfiguration.mockReturnValue({
      get: jest.fn().mockReturnValue("/Applications/Xcode-beta.app/Contents/Developer/usr/bin/xcodebuild"),
    });
    expect(getXcodeBuildCommand()).toBe("/Applications/Xcode-beta.app/Contents/Developer/usr/bin/xcodebuild");
  });

  it("expands environment variable in config value", () => {
    process.env.CUSTOM_XCODEBUILD = "/custom/path/to/xcodebuild";
    mockGetConfiguration.mockReturnValue({
      get: jest.fn().mockReturnValue("${env:CUSTOM_XCODEBUILD}"),
    });
    expect(getXcodeBuildCommand()).toBe("/custom/path/to/xcodebuild");
  });

  it("expands environment variable with additional path components", () => {
    process.env.XCODE_PATH = "/Applications/Xcode-beta.app";
    mockGetConfiguration.mockReturnValue({
      get: jest.fn().mockReturnValue("${env:XCODE_PATH}/Contents/Developer/usr/bin/xcodebuild"),
    });
    expect(getXcodeBuildCommand()).toBe("/Applications/Xcode-beta.app/Contents/Developer/usr/bin/xcodebuild");
  });

  it("keeps original placeholder when environment variable is not set", () => {
    process.env.NONEXISTENT_VAR = undefined;
    mockGetConfiguration.mockReturnValue({
      get: jest.fn().mockReturnValue("${env:NONEXISTENT_VAR}"),
    });
    expect(getXcodeBuildCommand()).toBe("${env:NONEXISTENT_VAR}");
  });

  it("expands empty environment variable to empty string", () => {
    process.env.EMPTY_VAR = "";
    mockGetConfiguration.mockReturnValue({
      get: jest.fn().mockReturnValue("prefix${env:EMPTY_VAR}suffix"),
    });
    expect(getXcodeBuildCommand()).toBe("prefixsuffix");
  });
});

describe("parseCliJsonOutput ", () => {
  it("simple", async () => {
    const input = `{"key1":"value1","key2":2}`;
    const obj = parseCliJsonOutput(input);
    expect(obj).toEqual({ key1: "value1", key2: 2 });
  });

  it("with noise", async () => {
    const input = `Some initial noise
{"key1":"value1","key2":2}
Some trailing noise`;
    const obj = parseCliJsonOutput(input);
    expect(obj).toEqual({ key1: "value1", key2: 2 });
  });

  it("multiple json objects", async () => {
    const input = `Noise before
{"key1":"value1"}
Some noise in between
{"key2":2}
Noise after`;
    expect(() => parseCliJsonOutput(input)).toThrow(ExtensionError);
  });

  it("no valid json", async () => {
    const input = `Just some random text
No JSON here!`;
    expect(() => parseCliJsonOutput(input)).toThrow(ExtensionError);
  });

  it("malformed json", async () => {
    const input = `Noise
{"key1":"value1", "key2":2
More noise`;
    expect(() => parseCliJsonOutput(input)).toThrow(ExtensionError);
  });

  it("json array", async () => {
    const input = `Noise
["item1", "item2", "item3"]
More noise`;
    const obj = parseCliJsonOutput(input);
    expect(obj).toEqual(["item1", "item2", "item3"]);
  });
  it("json array with nose and objects", async () => {
    const input = `Noise
[{"key1":"value1"}, {"key2":2}]
More noise`;
    const obj = parseCliJsonOutput(input);
    expect(obj).toEqual([{ key1: "value1" }, { key2: 2 }]);
  });
});
