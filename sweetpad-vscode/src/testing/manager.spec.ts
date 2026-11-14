import { beforeEach, describe, expect, it, vi } from "vitest";
import type * as vscode from "vscode";

import { errorReporting } from "../common/error-reporting";
import { commonLogger } from "../common/logger";
import { QuickPickCancelledError } from "../common/quick-pick";
import { TestingManager, findTestClasses } from "./manager";

vi.mock("../common/logger", () => ({
  commonLogger: { log: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
}));

vi.mock("../common/error-reporting", () => ({
  errorReporting: { captureException: vi.fn() },
}));

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

describe("TestingManager.createRunProfile", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  /** Registers a profile whose run fails with `error`, and calls the handler VS Code receives. */
  async function runFailingProfile(error: Error): Promise<unknown> {
    const manager = new TestingManager({} as ConstructorParameters<typeof TestingManager>[0]);
    manager.createRunProfile({
      name: "Build and Run Tests",
      kind: 1 satisfies vscode.TestRunProfileKind.Run,
      run: async () => {
        throw error;
      },
    });
    const handler = vi.mocked(manager.controller.createRunProfile).mock.calls[0][2];
    return await handler({} as vscode.TestRunRequest, {} as vscode.CancellationToken);
  }

  it("ends the run quietly when a picker is dismissed", async () => {
    await expect(runFailingProfile(new QuickPickCancelledError())).resolves.toBeUndefined();
    expect(commonLogger.error).not.toHaveBeenCalled();
    expect(errorReporting.captureException).not.toHaveBeenCalled();
  });

  it("logs, reports and rethrows any other error", async () => {
    const error = new Error("xcodebuild failed");
    await expect(runFailingProfile(error)).rejects.toBe(error);
    expect(commonLogger.error).toHaveBeenCalledWith("xcodebuild failed", { error: error });
    expect(errorReporting.captureException).toHaveBeenCalledWith(error);
  });
});
