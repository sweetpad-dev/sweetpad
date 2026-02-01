import { ExtensionError } from "../errors";
import { parseCliJsonOutput } from "./scripts";

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
