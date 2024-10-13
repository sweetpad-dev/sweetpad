import * as vscode from "vscode";
import { askScheme, askXcodeWorkspacePath } from "../build/utils.js";
import type { ExtensionContext } from "../common/commands";
import { runTask } from "../common/tasks.js";

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

function parseXcodebuildError(output: string, className: string, methodName: string): string {
  // Simple parsing logic; in a real scenario, you might want to use more robust parsing
  const lines = output.split("\n");
  let errorMessage = "Test failed";

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.includes(`-[${className} ${methodName}]`)) {
      // Found the test method
      // The next lines may contain the failure reason
      for (let j = i + 1; j < lines.length && j < i + 5; j++) {
        if (lines[j].includes("Assertion failed")) {
          errorMessage = lines[j].trim();
          break;
        }
      }
      break;
    }
  }

  return errorMessage;
}

export class TestingManager {
  controller: vscode.TestController;
  context: ExtensionContext;

  constructor(context: ExtensionContext) {
    this.context = context;

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

  dispose() {
    this.controller.dispose();
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

      // when class test is runned, all its method tests are runned too, so we need to filter them out from the queue
      queue = queue.filter((test) => {
        const [className, methodName] = test.id.split(".");
        if (!methodName) return true;
        return !queue.some((t) => t.id === className);
      });

      // todo: ask user for test target name
      const testTargetName = "ControlRoomTests";

      for (const test of queue) {
        console.log("Running test: ", test.id);
        if (token.isCancellationRequested) {
          run.skipped(test);
          continue;
        }

        if (test.id.includes(".")) {
          await this.runTestMethod({
            run: run,
            test: test,
            scheme: scheme,
            target: testTargetName,
          });
        } else {
          await this.runTestClass({
            run: run,
            test: test,
            scheme: scheme,
            target: testTargetName,
          });
        }
      }
    } finally {
      run.end();
    }
  }

  async runTestClass(options: {
    run: vscode.TestRun;
    test: vscode.TestItem;
    scheme: string;
    target: string;
  }): Promise<void> {
    const { run, test, scheme, target } = options;

    const className = test.id;
    run.started(test);

    const childrenMap = new Map<string, vscode.TestItem>();
    for (const [id, child] of test.children) {
      childrenMap.set(id, child);
    }

    // Keep track of tests that have been processed and their results
    const processedTests = new Set<string>();
    const failedTests = new Set<string>();

    try {
      const onlyTesting = `${target}/${test.id}`;

      await runTask(this.context, {
        name: "sweetpad.build.test",
        callback: async (terminal) => {
          await terminal.execute({
            command: "xcodebuild",
            args: ["test-without-building", "-scheme", scheme, `-only-testing:${onlyTesting}`],
            onOutputLine: async (output) => {
              const line = output.value.trim();
              if (!line.startsWith("Test Case")) {
                return;
              }

              const match = line.match(/Test Case '-\[(.*) (.*)\]' (.*)/);
              if (!match) {
                return;
              }

              const [, fullClassName, methodName, status] = match;
              const testId = `${className}.${methodName}`;
              const methodTestItem = childrenMap.get(testId);
              if (!methodTestItem) {
                return;
              }

              if (status.startsWith("started")) {
                run.started(methodTestItem);
              } else if (status.startsWith("passed")) {
                run.passed(methodTestItem);
                processedTests.add(testId);
              } else if (status.startsWith("failed")) {
                const errorMessage = parseXcodebuildError(output.value, className, methodName);
                run.failed(methodTestItem, new vscode.TestMessage(errorMessage));
                processedTests.add(testId);
                failedTests.add(testId);
              }
            },
          });
        },
      });
    } catch (error) {
      // Handle any errors during test execution
      const errorMessage = `Test class failed due to an error: ${error instanceof Error ? error.message : "Test failed"}`;
      run.failed(test, new vscode.TestMessage(errorMessage));

      // Mark all unprocessed child tests as failed
      for (const [testId, methodTestItem] of childrenMap.entries()) {
        if (!processedTests.has(testId)) {
          run.failed(methodTestItem, new vscode.TestMessage("Test failed due to an error."));
        }
      }
    } finally {
      // Mark any unprocessed tests as skipped
      for (const [testId, methodTestItem] of childrenMap.entries()) {
        if (!processedTests.has(testId)) {
          run.skipped(methodTestItem);
        }
      }

      // Determine the overall status of the test class
      if (failedTests.size > 0) {
        run.failed(test, new vscode.TestMessage("One or more tests failed."));
      } else if (processedTests.size === childrenMap.size) {
        run.passed(test);
      } else {
        run.skipped(test);
      }
    }
  }

  async runTestMethod(options: {
    run: vscode.TestRun;
    test: vscode.TestItem;
    scheme: string;
    target: string;
  }): Promise<void> {
    const { run, test, scheme, target } = options;
    const [className, methodName] = test.id.split(".");

    // Start the test
    run.started(test);

    let testResultReceived = false;

    // Run "xcodebuild" command as a task to see the test output
    await runTask(this.context, {
      name: "sweetpad.build.test",
      callback: async (terminal) => {
        try {
          const onlyTesting = `${target}/${className}/${methodName}`;

          await terminal.execute({
            command: "xcodebuild",
            args: ["test-without-building", "-scheme", scheme, `-only-testing:${onlyTesting}`],
            onOutputLine: async (output) => {
              const line = output.value.trim();

              // not a test method line, skip
              if (!line.startsWith("Test Case")) {
                return;
              }

              // Example output:
              // "Test Case '-[ControlRoomTests.SimCtlSubCommandsTests testDeleteUnavailable]' started."
              // "Test Case '-[ControlRoomTests.SimCtlSubCommandsTests testDeleteUnavailable]' passed (0.001 seconds)."
              // "Test Case '-[ControlRoomTests.SimCtlSubCommandsTests testDeleteUnavailable]' failed (0.001 seconds)."
              const match = line.match(/Test Case '-\[(.*) (.*)\]' (.*)/);
              if (!match) {
                return;
              }

              const [, , testMethodName, status] = match;

              // Not our test method, skip
              if (testMethodName !== methodName) {
                return;
              }

              if (status.startsWith("started")) {
                // Test already started
              } else if (status.startsWith("passed")) {
                run.passed(test);
                testResultReceived = true;
              } else if (status.startsWith("failed")) {
                // Optionally parse error message
                const errorMessage = "Test failed"; // You can enhance this by parsing the error output
                run.failed(test, new vscode.TestMessage(errorMessage));
                testResultReceived = true;
              }
            },
          });
        } catch (error) {
          // todo: proper error handling
          const errorMessage = error instanceof Error ? error.message : "Test failed";
          run.failed(test, new vscode.TestMessage(errorMessage));
        } finally {
          if (!testResultReceived) {
            run.skipped(test);
          }
        }
      },
    });
  }
}
