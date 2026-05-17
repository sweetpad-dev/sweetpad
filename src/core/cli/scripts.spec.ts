import type { ConfigKey, ConfigProvider, ConfigSchema } from "../config/types";
import { ExtensionError } from "../errors";
import { noopLogger } from "../logger/types";
import { getXcodeBuildCommand, parseCliJsonOutput, type XcodeCliDeps } from "./scripts";

function makeConfig(values: Partial<{ [K in ConfigKey]: ConfigSchema[K] }>): ConfigProvider {
  return {
    get<K extends ConfigKey>(key: K): ConfigSchema[K] | undefined {
      return values[key] as ConfigSchema[K] | undefined;
    },
    isDefined<K extends ConfigKey>(key: K): boolean {
      return values[key] !== undefined;
    },
    async update() {
      // no-op for tests
    },
  };
}

function makeDeps(config: ConfigProvider): XcodeCliDeps {
  return { cwd: "/tmp", config, logger: noopLogger };
}

describe("getXcodeBuildCommand", () => {
  const originalEnv = process.env;

  beforeEach(() => {
    process.env = { ...originalEnv };
  });

  afterAll(() => {
    process.env = originalEnv;
  });

  it("returns default 'xcodebuild' when no config is set", () => {
    const deps = makeDeps(makeConfig({}));
    expect(getXcodeBuildCommand(deps)).toBe("xcodebuild");
  });

  it("returns default 'xcodebuild' when config is empty string", () => {
    const deps = makeDeps(makeConfig({ "build.xcodebuildCommand": "" }));
    expect(getXcodeBuildCommand(deps)).toBe("xcodebuild");
  });

  it("returns custom command when configured with absolute path", () => {
    const deps = makeDeps(
      makeConfig({
        "build.xcodebuildCommand": "/Applications/Xcode-beta.app/Contents/Developer/usr/bin/xcodebuild",
      }),
    );
    expect(getXcodeBuildCommand(deps)).toBe("/Applications/Xcode-beta.app/Contents/Developer/usr/bin/xcodebuild");
  });
});

describe("parseCliJsonOutput", () => {
  it("simple", () => {
    const input = `{"key1":"value1","key2":2}`;
    const obj = parseCliJsonOutput(input, noopLogger);
    expect(obj).toEqual({ key1: "value1", key2: 2 });
  });

  it("with noise", () => {
    const input = `Some initial noise
{"key1":"value1","key2":2}
Some trailing noise`;
    const obj = parseCliJsonOutput(input, noopLogger);
    expect(obj).toEqual({ key1: "value1", key2: 2 });
  });

  it("multiple json objects", () => {
    const input = `Noise before
{"key1":"value1"}
Some noise in between
{"key2":2}
Noise after`;
    expect(() => parseCliJsonOutput(input, noopLogger)).toThrow(ExtensionError);
  });

  it("no valid json", () => {
    const input = `Just some random text
No JSON here!`;
    expect(() => parseCliJsonOutput(input, noopLogger)).toThrow(ExtensionError);
  });

  it("malformed json", () => {
    const input = `Noise
{"key1":"value1", "key2":2
More noise`;
    expect(() => parseCliJsonOutput(input, noopLogger)).toThrow(ExtensionError);
  });

  it("json array", () => {
    const input = `Noise
["item1", "item2", "item3"]
More noise`;
    const obj = parseCliJsonOutput(input, noopLogger);
    expect(obj).toEqual(["item1", "item2", "item3"]);
  });

  it("json array with noise and objects", () => {
    const input = `Noise
[{"key1":"value1"}, {"key2":2}]
More noise`;
    const obj = parseCliJsonOutput(input, noopLogger);
    expect(obj).toEqual([{ key1: "value1" }, { key2: 2 }]);
  });
});
