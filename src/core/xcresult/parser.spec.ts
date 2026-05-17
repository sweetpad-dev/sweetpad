import * as child_process from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { type Mock, afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { noopLogger } from "../logger/types";
import { parseXcresultBundle } from "./parser";

vi.mock("node:child_process", async () => {
  const actual = await vi.importActual<typeof import("node:child_process")>("node:child_process");
  return {
    ...actual,
    execFile: vi.fn(),
  };
});

const mockedExecFile = child_process.execFile as unknown as Mock;

function setupBundle(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-xcresult-"));
  const bundlePath = path.join(dir, "Scheme.xcresult");
  fs.mkdirSync(bundlePath);
  return bundlePath;
}

describe("parseXcresultBundle", () => {
  beforeEach(() => {
    mockedExecFile.mockReset();
  });

  afterEach(() => {
    // tmp dirs are tiny — fine to leak across tests
  });

  it("returns undefined when no bundle exists at either candidate path", async () => {
    const result = await parseXcresultBundle("/nonexistent/path", { logger: noopLogger });
    expect(result).toBeUndefined();
  });

  it("parses counts from xcresulttool summary output", async () => {
    const bundlePath = setupBundle();

    mockedExecFile
      .mockImplementationOnce((_cmd, args, _opts, cb) => {
        // first call: summary
        cb?.(null, {
          stdout: JSON.stringify({
            result: "Failed",
            totalTestCount: 10,
            passedTests: 7,
            failedTests: 2,
            skippedTests: 1,
          }),
          stderr: "",
        });
        return { stdout: "", stderr: "" } as unknown as ReturnType<typeof child_process.execFile>;
      })
      .mockImplementationOnce((_cmd, _args, _opts, cb) => {
        // second call: tests
        cb?.(null, { stdout: JSON.stringify({ testNodes: [] }), stderr: "" });
        return { stdout: "", stderr: "" } as unknown as ReturnType<typeof child_process.execFile>;
      });

    const result = await parseXcresultBundle(bundlePath, { logger: noopLogger });
    expect(result).toEqual({
      totalTestCount: 10,
      passedTests: 7,
      failedTests: 2,
      skippedTests: 1,
      testCases: [],
    });
  });

  it("flattens nested testNodes down to a flat TestCaseSummary list", async () => {
    const bundlePath = setupBundle();

    mockedExecFile
      .mockImplementationOnce((_cmd, _args, _opts, cb) => {
        cb?.(null, {
          stdout: JSON.stringify({
            totalTestCount: 2,
            passedTests: 1,
            failedTests: 1,
            skippedTests: 0,
          }),
          stderr: "",
        });
        return {} as unknown as ReturnType<typeof child_process.execFile>;
      })
      .mockImplementationOnce((_cmd, _args, _opts, cb) => {
        cb?.(null, {
          stdout: JSON.stringify({
            testNodes: [
              {
                nodeType: "Test Plan",
                name: "MyAppTests",
                children: [
                  {
                    nodeType: "Test Suite",
                    name: "FooSuite",
                    children: [
                      {
                        nodeType: "Test Case",
                        name: "testAlpha",
                        identifier: "FooSuite/testAlpha",
                        result: "Passed",
                        duration: "0.234s",
                      },
                      {
                        nodeType: "Test Case",
                        name: "testBeta",
                        identifier: "FooSuite/testBeta",
                        result: "Failed",
                        duration: "1.5s",
                        failureMessages: [{ message: "expected true, got false" }],
                      },
                    ],
                  },
                ],
              },
            ],
          }),
          stderr: "",
        });
        return {} as unknown as ReturnType<typeof child_process.execFile>;
      });

    const result = await parseXcresultBundle(bundlePath, { logger: noopLogger });
    expect(result?.testCases).toEqual([
      { identifier: "FooSuite/testAlpha", status: "passed", durationMs: 234, message: undefined },
      { identifier: "FooSuite/testBeta", status: "failed", durationMs: 1500, message: "expected true, got false" },
    ]);
  });

  it("returns counts but empty testCases when the tests endpoint fails", async () => {
    const bundlePath = setupBundle();

    mockedExecFile
      .mockImplementationOnce((_cmd, _args, _opts, cb) => {
        cb?.(null, {
          stdout: JSON.stringify({ totalTestCount: 1, passedTests: 1, failedTests: 0, skippedTests: 0 }),
          stderr: "",
        });
        return {} as unknown as ReturnType<typeof child_process.execFile>;
      })
      .mockImplementationOnce((_cmd, _args, _opts, cb) => {
        cb?.(new Error("xcresulttool: unknown command 'tests'"), { stdout: "", stderr: "" });
        return {} as unknown as ReturnType<typeof child_process.execFile>;
      });

    const result = await parseXcresultBundle(bundlePath, { logger: noopLogger });
    expect(result?.totalTestCount).toBe(1);
    expect(result?.testCases).toEqual([]);
  });

  it("returns undefined when the summary call errors", async () => {
    const bundlePath = setupBundle();
    mockedExecFile.mockImplementationOnce((_cmd, _args, _opts, cb) => {
      cb?.(new Error("xcrun: command not found"), { stdout: "", stderr: "" });
      return {} as unknown as ReturnType<typeof child_process.execFile>;
    });
    const result = await parseXcresultBundle(bundlePath, { logger: noopLogger });
    expect(result).toBeUndefined();
  });

  it("tries `<path>.xcresult` when the bare path doesn't exist", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-xcresult-"));
    const base = path.join(dir, "Scheme");
    // Note: not creating `base`, only `base.xcresult`
    fs.mkdirSync(`${base}.xcresult`);

    mockedExecFile
      .mockImplementationOnce((_cmd, args: string[], _opts, cb) => {
        expect(args).toContain(`${base}.xcresult`);
        cb?.(null, {
          stdout: JSON.stringify({ totalTestCount: 0, passedTests: 0, failedTests: 0, skippedTests: 0 }),
          stderr: "",
        });
        return {} as unknown as ReturnType<typeof child_process.execFile>;
      })
      .mockImplementationOnce((_cmd, _args, _opts, cb) => {
        cb?.(null, { stdout: JSON.stringify({ testNodes: [] }), stderr: "" });
        return {} as unknown as ReturnType<typeof child_process.execFile>;
      });

    const result = await parseXcresultBundle(base, { logger: noopLogger });
    expect(result).not.toBeUndefined();
  });
});
