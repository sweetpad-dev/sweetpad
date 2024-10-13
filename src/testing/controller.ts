import * as vscode from "vscode";
import { askScheme, askXcodeWorkspacePath } from "../build/utils.js";
import type { ExtensionContext } from "../common/commands";
import { commonLogger } from "../common/logger.js";
import { runTask } from "../common/tasks.js";
import { askTestingTarget } from "./utils.js";

type TestingInlineError = {
  fileName: string;
  lineNumber: number;
  message: string;
};

/**
 * Track the result of each `xcodebuild` test run — which tests have been processed, failed and so on.
 *
 * - methodTestId: the test method ID in the format "ClassName.methodName"
 */
class XcodebuildTestRunContext {
  private processedMethodTests = new Set<string>();
  private failedMethodTests = new Set<string>();
  private inlineErrorMap = new Map<string, TestingInlineError>();
  private methodTests: Map<string, vscode.TestItem>;

  constructor(options: {
    methodTests: Iterable<[string, vscode.TestItem]>;
  }) {
    this.methodTests = new Map(options.methodTests);
  }

  getMethodTest(methodTestId: string): vscode.TestItem | undefined {
    return this.methodTests.get(methodTestId);
  }

  addProcessedMethodTest(methodTestId: string): void {
    this.processedMethodTests.add(methodTestId);
  }

  addFailedMethodTest(methodTestId: string): void {
    this.failedMethodTests.add(methodTestId);
  }

  addInlineError(methodTestId: string, error: TestingInlineError): void {
    this.inlineErrorMap.set(methodTestId, error);
  }

  getInlineError(methodTestId: string): TestingInlineError | undefined {
    return this.inlineErrorMap.get(methodTestId);
  }

  isMethodTestProcessed(methodTestId: string): boolean {
    return this.processedMethodTests.has(methodTestId);
  }

  getUnprocessedMethodTests(): vscode.TestItem[] {
    return [...this.methodTests.values()].filter((test) => !this.processedMethodTests.has(test.id));
  }

  getOverallStatus(): "passed" | "failed" | "skipped" {
    // Some tests failed
    if (this.failedMethodTests.size > 0) {
      return "failed";
    }

    // All tests passed
    if (this.processedMethodTests.size === this.methodTests.size) {
      return "passed";
    }

    // Some tests are still unprocessed
    return "skipped";
  }
}

/**
 * Extracts a code block from the given text starting from the given index.
 *
 * TODO: use a proper Swift parser to find code blocks
 */
function extractCodeBlock(text: string, startIndex: number): string | null {
  let braceCount = 0;
  let inString = false;
  for (let i = startIndex; i < text.length; i++) {
    const char = text[i];
    if (char === '"' || char === "'") {
      inString = !inString;
    } else if (!inString) {
      if (char === "{") {
        braceCount++;
      } else if (char === "}") {
        braceCount--;
        if (braceCount === 0) {
          return text.substring(startIndex, i + 1);
        }
      }
    }
  }
  return null;
}

export class TestingManager {
  controller: vscode.TestController;
  private _context: ExtensionContext | undefined;

  // Inline error messages, usually is between "passed" and "failed" lines
  // Example output:
  // "/Users/username/Projects/ControlRoom/ControlRoomTests/SimCtlSubCommandsTests.swift:10: error: -[ControlRoomTests.SimCtlSubCommandsTests testDeleteUnavailable] : failed: caught "NSInternalInconsistencyException", "Failed to delete unavailable device with UDID '00000000-0000-0000-0000-000000000000'."
  // "/Users/hyzyla/Developer/sweetpad-examples/ControlRoom/ControlRoomTests/Controllers/SimCtl+SubCommandsTests.swift:76: error: -[ControlRoomTests.SimCtlSubCommandsTests testDefaultsForApp] : XCTAssertEqual failed: ("1") is not equal to ("2")"
  // {filePath}:{lineNumber}: error: -[{classAndTargetName} {methodName}] : {errorMessage}
  readonly INLINE_ERROR_REGEXP = /(.*):(\d+): error: -\[.* (.*)\] : (.*)/;

  // Find test method status lines
  // Example output:
  // "Test Case '-[ControlRoomTests.SimCtlSubCommandsTests testDeleteUnavailable]' started."
  // "Test Case '-[ControlRoomTests.SimCtlSubCommandsTests testDeleteUnavailable]' passed (0.001 seconds)."
  // "Test Case '-[ControlRoomTests.SimCtlSubCommandsTests testDeleteUnavailable]' failed (0.001 seconds)."
  readonly METHOD_STATUS_REGEXP = /Test Case '-\[(.*) (.*)\]' (.*)/;

  constructor() {
    this.controller = vscode.tests.createTestController("sweetpad", "SweetPad");

    // Register event listeners for updating test items when documents change or open
    vscode.workspace.onDidOpenTextDocument((document) => this.updateTestItems(document));
    vscode.workspace.onDidChangeTextDocument((event) => this.updateTestItems(event.document));

    // Initialize test items for already open documents
    for (const document of vscode.workspace.textDocuments) {
      this.updateTestItems(document);
    }

    // Add handler for running tests
    this.controller.createRunProfile("Run Tests", vscode.TestRunProfileKind.Run, (request, token) => {
      return this.runTest(request, token);
    });
  }

  set context(context: ExtensionContext) {
    this._context = context;
  }

  get context(): ExtensionContext {
    if (!this._context) {
      throw new Error("Context is not set");
    }
    return this._context;
  }

  dispose() {
    this.controller.dispose();
  }

  setDefaultTestingTarget(target: string | undefined) {
    this.context.updateWorkspaceState("testing.xcodeTarget", target);
  }

  getDefaultTestingTarget(): string | undefined {
    return this.context.getWorkspaceState("testing.xcodeTarget");
  }

  /**
   * Find all test methods in the given document and update the test items in test controller
   *
   * TODO: use a proper Swift parser to find test methods
   */
  updateTestItems(document: vscode.TextDocument) {
    // Remove existing test items for this document
    for (const testItem of this.controller.items) {
      if (testItem[1].uri?.toString() === document.uri.toString()) {
        this.controller.items.delete(testItem[0]);
      }
    }

    // Check if this is a Swift file
    if (!document.fileName.endsWith(".swift")) {
      return;
    }

    const text = document.getText();

    // Regex to find classes inheriting from XCTestCase
    const classRegex = /class\s+(\w+)\s*:\s*XCTestCase\s*\{/g;
    // let classMatch;
    while (true) {
      const classMatch = classRegex.exec(text);
      if (classMatch === null) {
        break;
      }
      const className = classMatch[1];
      const classStartIndex = classMatch.index + classMatch[0].length;
      const classPosition = document.positionAt(classMatch.index);

      const classTestItem = this.controller.createTestItem(className, className, document.uri);
      classTestItem.range = new vscode.Range(classPosition, classPosition);
      this.controller.items.add(classTestItem);
      //   allItems.set(className, classTestItem);

      const classCode = extractCodeBlock(text, classStartIndex - 1); // Start from '{'

      if (classCode === null) {
        continue; // Could not find class code block
      }

      // Find all test methods within the class
      const funcRegex = /func\s+(test\w+)\s*\(/g;

      while (true) {
        const funcMatch = funcRegex.exec(classCode);
        if (funcMatch === null) {
          break;
        }
        const testName = funcMatch[1];
        const testStartIndex = classStartIndex + funcMatch.index;
        const position = document.positionAt(testStartIndex);

        const testItem = this.controller.createTestItem(`${className}.${testName}`, testName, document.uri);
        testItem.range = new vscode.Range(position, position);
        classTestItem.children.add(testItem);
        // allItems.set(`${className}.${testName}`, testItem);
      }
    }
  }

  /**
   * Build the project for testing
   */
  async buildForTesting(options: {
    scheme: string;
  }) {
    await runTask(this.context, {
      name: "sweetpad.build.build",
      callback: async (terminal) => {
        await terminal.execute({
          command: "xcodebuild",
          args: ["build-for-testing", "-scheme", options.scheme],
        });
      },
    });
  }

  /**
   * Parse each line of the `xcodebuild` output to update the test run
   * with the test status and any inline error messages.
   */
  async parseOutputLine(options: {
    line: string;
    className: string;
    testRun: vscode.TestRun;
    runContext: XcodebuildTestRunContext;
  }) {
    const { testRun, className, runContext } = options;
    const line = options.line.trim();

    const methodStatusMatch = line.match(this.METHOD_STATUS_REGEXP);
    if (methodStatusMatch) {
      const [, , methodName, status] = methodStatusMatch;
      const methodTestId = `${className}.${methodName}`;

      const methodTest = runContext.getMethodTest(methodTestId);
      if (!methodTest) {
        return;
      }

      if (status.startsWith("started")) {
        testRun.started(methodTest);
      } else if (status.startsWith("passed")) {
        testRun.passed(methodTest);
        runContext.addProcessedMethodTest(methodTestId);
      } else if (status.startsWith("failed")) {
        // Inline error message are usually before the "failed" line
        const error = runContext.getInlineError(methodTestId);
        if (error) {
          // detailed error message with location
          const testMessage = new vscode.TestMessage(error.message);
          testMessage.location = new vscode.Location(
            vscode.Uri.file(error.fileName),
            new vscode.Position(error.lineNumber - 1, 0),
          );
          testRun.failed(methodTest, testMessage);
        } else {
          // just geenric error message, no error location or details
          testRun.failed(methodTest, new vscode.TestMessage("Test failed"));
        }

        runContext.addProcessedMethodTest(methodTestId);
        runContext.addFailedMethodTest(methodTestId);
      }
      return;
    }

    const inlineErrorMatch = line.match(this.INLINE_ERROR_REGEXP);
    if (inlineErrorMatch) {
      const [, filePath, lineNumber, methodName, errorMessage] = inlineErrorMatch;
      const testId = `${className}.${methodName}`;
      runContext.addInlineError(testId, {
        fileName: filePath,
        lineNumber: Number.parseInt(lineNumber, 10),
        message: errorMessage,
      });
      return;
    }
  }

  async runTest(request: vscode.TestRunRequest, token: vscode.CancellationToken) {
    const run = this.controller.createTestRun(request);
    try {
      let queue: vscode.TestItem[] = [];

      if (request.include) {
        // all tests selected by the user
        queue.push(...request.include);
      } else {
        // all root test items
        queue.push(...[...this.controller.items].map(([, item]) => item));
      }

      const xcworkspace = await askXcodeWorkspacePath(this.context);
      const scheme = await askScheme(this.context, {
        xcworkspace: xcworkspace,
        title: "Select a scheme to run tests",
      });

      // before testing we need to build the project to avoid runnning tests on old code or
      // building every time we run selected tests
      await this.buildForTesting({ scheme: scheme });

      // when class test is runned, all its method tests are runned too, so we need to filter out
      // methods that should be runned as part of class test
      queue = queue.filter((test) => {
        const [className, methodName] = test.id.split(".");
        if (!methodName) return true;
        return !queue.some((t) => t.id === className);
      });

      const testTargetName = await askTestingTarget(this.context, {
        xcworkspace: xcworkspace,
        title: "Select a target to run tests",
      });

      commonLogger.debug("Running tests", {
        scheme: scheme,
        target: testTargetName,
        xcworkspace: xcworkspace,
        tests: queue.map((test) => test.id),
      });

      for (const test of queue) {
        commonLogger.debug("Running single test from queue", {
          testId: test.id,
          testLabel: test.label,
        });

        if (token.isCancellationRequested) {
          run.skipped(test);
          continue;
        }

        if (test.id.includes(".")) {
          await this.runMethodTest({
            run: run,
            methodTest: test,
            scheme: scheme,
            target: testTargetName,
          });
        } else {
          await this.runClassTest({
            run: run,
            classTest: test,
            scheme: scheme,
            target: testTargetName,
          });
        }
      }
    } finally {
      run.end();
    }
  }

  async runClassTest(options: {
    run: vscode.TestRun;
    classTest: vscode.TestItem;
    scheme: string;
    target: string;
  }): Promise<void> {
    const { run, classTest, scheme, target } = options;
    const className = classTest.id;

    const runContext = new XcodebuildTestRunContext({
      methodTests: [...classTest.children],
    });

    run.started(classTest);

    try {
      await runTask(this.context, {
        name: "sweetpad.build.test",
        callback: async (terminal) => {
          await terminal.execute({
            command: "xcodebuild",
            args: ["test-without-building", "-scheme", scheme, `-only-testing:${target}/${classTest.id}`],
            onOutputLine: async (output) => {
              await this.parseOutputLine({
                line: output.value,
                testRun: run,
                className: className,
                runContext: runContext,
              });
            },
          });
        },
      });
    } catch (error) {
      // Handle any errors during test execution
      const errorMessage = `Test class failed due to an error: ${error instanceof Error ? error.message : "Test failed"}`;
      run.failed(classTest, new vscode.TestMessage(errorMessage));

      // Mark all unprocessed child tests as failed
      for (const methodTest of runContext.getUnprocessedMethodTests()) {
        run.failed(methodTest, new vscode.TestMessage("Test failed due to an error."));
      }
    } finally {
      // Mark any unprocessed tests as skipped
      for (const methodTest of runContext.getUnprocessedMethodTests()) {
        run.skipped(methodTest);
      }

      // Determine the overall status of the test class
      const overallStatus = runContext.getOverallStatus();
      if (overallStatus === "failed") {
        run.failed(classTest, new vscode.TestMessage("One or more tests failed."));
      } else if (overallStatus === "passed") {
        run.passed(classTest);
      } else if (overallStatus === "skipped") {
        run.skipped(classTest);
      }
    }
  }

  async runMethodTest(options: {
    run: vscode.TestRun;
    methodTest: vscode.TestItem;
    scheme: string;
    target: string;
  }): Promise<void> {
    const { run: testRun, methodTest, scheme, target } = options;
    const [className, methodName] = methodTest.id.split(".");

    const runContext = new XcodebuildTestRunContext({
      methodTests: [[methodTest.id, methodTest]],
    });

    // Run "xcodebuild" command as a task to see the test output
    await runTask(this.context, {
      name: "sweetpad.build.test",
      callback: async (terminal) => {
        try {
          await terminal.execute({
            command: "xcodebuild",
            args: ["test-without-building", "-scheme", scheme, `-only-testing:${target}/${className}/${methodName}`],
            onOutputLine: async (output) => {
              await this.parseOutputLine({
                line: output.value,
                testRun: testRun,
                className: className,
                runContext: runContext,
              });
            },
          });
        } catch (error) {
          // todo: ??? can we extract error message from error object?
          const errorMessage = error instanceof Error ? error.message : "Test failed";
          testRun.failed(methodTest, new vscode.TestMessage(errorMessage));
        } finally {
          if (!runContext.isMethodTestProcessed(methodTest.id)) {
            testRun.skipped(methodTest);
          }
        }
      },
    });
  }
}
