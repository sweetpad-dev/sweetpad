import { describe, expect, it } from "vitest";

import { findTestClasses } from "./manager";

const DEFAULT_BASES = new Set(["XCTestCase"]);

/** Class names found in `src`, in source order. */
function names(src: string, bases: Set<string> = DEFAULT_BASES): string[] {
  return findTestClasses(src, bases).map((match) => match.className);
}

describe("findTestClasses", () => {
  it("finds a class inheriting from XCTestCase directly", () => {
    expect(names("class FooTests: XCTestCase {\n}")).toEqual(["FooTests"]);
  });

  it("finds a class inheriting from a configured base class", () => {
    const src = "class FooTests: BaseTestCase {\n}";
    expect(names(src)).toEqual([]);
    expect(names(src, new Set(["XCTestCase", "BaseTestCase"]))).toEqual(["FooTests"]);
  });

  it("ignores a class inheriting from an unknown type", () => {
    expect(names("class ViewModel: ObservableObject {\n}")).toEqual([]);
  });

  it("finds a class that also conforms to protocols", () => {
    expect(names("final class FooTests: XCTestCase, Sendable {\n}")).toEqual(["FooTests"]);
  });

  it("finds a class whose inheritance clause is wrapped across lines", () => {
    expect(names("class FooTests:\n    XCTestCase\n{\n}")).toEqual(["FooTests"]);
  });

  it("finds a generic test class", () => {
    expect(names("class FooTests<Subject>: XCTestCase {\n}")).toEqual(["FooTests"]);
  });

  it("accepts a module-qualified base class", () => {
    expect(names("class FooTests: XCTest.XCTestCase {\n}")).toEqual(["FooTests"]);
  });

  it("looks past attributes and access modifiers", () => {
    expect(names("@MainActor\npublic final class FooTests: XCTestCase {\n}")).toEqual(["FooTests"]);
  });

  it("does not match a type whose name merely ends in 'class'", () => {
    expect(names("let subclass: XCTestCase\n")).toEqual([]);
  });

  it("skips a declaration with no body", () => {
    expect(names("class FooTests: XCTestCase")).toEqual([]);
  });

  it("finds every test class in a file, leaving other classes out", () => {
    const src = ["class Helper: NSObject {}", "class FooTests: XCTestCase {}", "class BarTests: BaseTestCase {}"].join(
      "\n",
    );
    expect(names(src, new Set(["XCTestCase", "BaseTestCase"]))).toEqual(["FooTests", "BarTests"]);
  });

  it("reports the declaration and body offsets", () => {
    const src = "// header\nclass FooTests: XCTestCase, Sendable {\n}";
    expect(findTestClasses(src, DEFAULT_BASES)).toEqual([
      {
        className: "FooTests",
        declarationIndex: src.indexOf("class FooTests"),
        bodyIndex: src.indexOf("{"),
      },
    ]);
  });
});
