import { uniqueFilter, prepareEnvVars } from "./helpers";

describe("uniqueFilter", () => {
  it("removes duplicate values from array", () => {
    expect([1, 2, 2, 3, 3, 3].filter(uniqueFilter)).toEqual([1, 2, 3]);
  });

  it("keeps unique values unchanged", () => {
    expect([1, 2, 3].filter(uniqueFilter)).toEqual([1, 2, 3]);
  });

  it("works with strings", () => {
    expect(["a", "b", "a", "c", "b"].filter(uniqueFilter)).toEqual(["a", "b", "c"]);
  });

  it("returns empty array for empty input", () => {
    expect([].filter(uniqueFilter)).toEqual([]);
  });

  it("keeps first occurrence of duplicates", () => {
    const result = ["first", "second", "first"].filter(uniqueFilter);
    expect(result).toEqual(["first", "second"]);
  });
});

describe("prepareEnvVars", () => {
  it("returns empty object for undefined input", () => {
    expect(prepareEnvVars(undefined)).toEqual({});
  });

  it("passes through string values", () => {
    expect(prepareEnvVars({ FOO: "bar", BAZ: "qux" })).toEqual({ FOO: "bar", BAZ: "qux" });
  });

  it("converts null values to undefined", () => {
    const result = prepareEnvVars({ FOO: "bar", BAZ: null });
    expect(result).toEqual({ FOO: "bar", BAZ: undefined });
    expect("BAZ" in result).toBe(true);
  });

  it("handles empty object", () => {
    expect(prepareEnvVars({})).toEqual({});
  });

  it("handles all null values", () => {
    const result = prepareEnvVars({ A: null, B: null });
    expect(result).toEqual({ A: undefined, B: undefined });
  });
});
