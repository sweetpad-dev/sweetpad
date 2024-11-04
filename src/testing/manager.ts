import * as vscode from "vscode";
import path from "path";
import { getXcodeBuildDestinationString } from "../build/commands.js";
import { askXcodeWorkspacePath, getWorkspacePath } from "../build/utils.js";
import { getBuildSettings } from "../common/cli/scripts.js";
import type { ExtensionContext } from "../common/commands.js";
import { commonLogger } from "../common/logger.js";
import { runTask } from "../common/tasks.js";
import type { Destination } from "../destination/types.js";
import {
  askConfigurationForTesting,
  askDestinationToTestOn,
  askSchemeForTesting,
  extractCodeBlock,
  parseDefaultTestPlanFile
} from "./utils.js";
import { findFilesRecursive } from '../common/files.js';

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
  private readonly processedMethodTests = new Set<string>();
  private readonly failedMethodTests = new Set<string>();
  private readonly inlineErrorMap = new Map<string, TestingInlineError>();
  private readonly methodTests: Map<string, vscode.TestItem>;
  // we need this mapping to find method test by its name without target name when running all tests
  private readonly methodWithoutTargetNameTests: Map<string, vscode.TestItem>;

  constructor(options: {
    methodTests: Iterable<[string, vscode.TestItem]>;
  }) {
    this.methodTests = new Map(options.methodTests);
    this.methodWithoutTargetNameTests = new Map(Array.from(options.methodTests).map(([id, test]) => {
      const [, className, methodName] = id.split(".");
      return [`${className}.${methodName}`, test]
    }))
  }

  getMethodTest(methodTestId: string): vscode.TestItem | undefined {
    return this.methodTests.get(methodTestId) ?? this.methodWithoutTargetNameTests.get(methodTestId);
  }

  addProcessedMethodTest(methodTestId: string): void {
    this.processedMethodTests.add(methodTestId);
  }

  addFailedMethodTest(methodTestId: string): void {
    this.failedMethodTests.add(methodTestId);
  }

  addInlineError(methodTestId: string, error: TestingInlineError): void {
    commonLogger.log("Adding inline error", {
      methodTestId,
      error,
    })
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
}

export class TestingManager {
  controller: vscode.TestController;
  private _context: ExtensionContext | undefined;
  private readonly documentToTargetTestItem = new Map<string, vscode.TestItem>();
  private readonly pathToTargetTestItem: [string, vscode.TestItem][] = [];
  private readonly uriToTestItem = new Map<string, vscode.TestItem[]>();

  // Inline error messages, usually is between "passed" and "failed" lines. Seems like only macOS apps have this line.
  // Example output:
  // "/Users/username/Projects/ControlRoom/ControlRoomTests/SimCtlSubCommandsTests.swift:10: error: -[ControlRoomTests.SimCtlSubCommandsTests testDeleteUnavailable] : failed: caught "NSInternalInconsistencyException", "Failed to delete unavailable device with UDID '00000000-0000-0000-0000-000000000000'."
  // "/Users/hyzyla/Developer/sweetpad-examples/ControlRoom/ControlRoomTests/Controllers/SimCtl+SubCommandsTests.swift:76: error: -[ControlRoomTests.SimCtlSubCommandsTests testDefaultsForApp] : XCTAssertEqual failed: ("1") is not equal to ("2")"
  // {filePath}:{lineNumber}: error: -[{classAndTargetName} {methodName}] : {errorMessage}
  readonly INLINE_ERROR_REGEXP = /(.*):(\d+): error: -\[(.*) (.*)\] : (.*)/;

  // Find test method status lines
  // Example output:
  // "Test Case '-[ControlRoomTests.SimCtlSubCommandsTests testDeleteUnavailable]' started."
  // "Test Case '-[ControlRoomTests.SimCtlSubCommandsTests testDeleteUnavailable]' passed (0.001 seconds)."
  // "Test Case '-[ControlRoomTests.SimCtlSubCommandsTests testDeleteUnavailable]' failed (0.001 seconds).")
  readonly METHOD_STATUS_REGEXP_MACOS = /Test Case '-\[(.*) (.*)\]' (.*)/;

  // "Test case 'terminal23TesMakarenko1ts.testExample1()' failed on 'Clone 1 of iPhone 14 - terminal23 (27767)' (0.154 seconds)"
  // "Test case 'terminal23TesMakarenko1ts.testExample2()' passed on 'Clone 1 of iPhone 14 - terminal23 (27767)' (0.000 seconds)"
  // "Test case 'terminal23TesMakarenko1ts.testPerformanceExample()' passed on 'Clone 1 of iPhone 14 - terminal23 (27767)' (0.254 seconds)"
  readonly METHOD_STATUS_REGEXP_IOS = /Test case '(.*)\.(.*)\(\)' (.*)/;

  constructor() {
    this.controller = vscode.tests.createTestController("sweetpad", "SweetPad");

    vscode.workspace.onDidCreateFiles(event => this.handleFileCreation(event))
    vscode.workspace.onDidDeleteFiles(event => this.handleFileDeletion(event))
    vscode.workspace.onDidChangeTextDocument(event => this.handleFileChange(event));

    // Default for profile that is slow due to build step, but should work in most cases
    this.controller.createRunProfile(
      "Build and Run Tests",
      vscode.TestRunProfileKind.Run,
      (request, token) => {
        return this.buildAndRunTests(request, token);
      },
      true, // is default profile
    );

    // Profile for running tests without building, should be faster but you may need to build manually
    this.controller.createRunProfile(
      "Run Tests Without Building",
      vscode.TestRunProfileKind.Run,
      (request, token) => {
        return this.runTestsWithoutBuilding(request, token);
      },
      false,
    );
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

  private handleFileCreation(event: vscode.FileCreateEvent) {
    event.files.forEach((file) => {
      if (!file.path.endsWith(".swift")) {
        return
      }

      const { path: documentPath } = file
      const targetTestItem = this.pathToTargetTestItem.find(([path,]) => documentPath.startsWith(path))?.[1]
      if (!targetTestItem) {
        return
      }

      this.documentToTargetTestItem.set(documentPath, targetTestItem)
      vscode.workspace.openTextDocument(file).then((file) => {
        this.updateTestItems(file, targetTestItem)
      })
    });
  }

  private handleFileDeletion(event: vscode.FileDeleteEvent) {
    event.files.forEach((file) => {
      if (!file.path.endsWith(".swift")) {
        return
      }

      const { path: documentPath } = file
      const targetTestItem = this.uriToTestItem.get(documentPath)
      if (!targetTestItem) {
        commonLogger.warn("No target test item found for the document", {
          documentPath,
          uriToTestItem: Array.from(this.uriToTestItem.keys())
        })
        return
      }

      for (const testItem of targetTestItem) {
        commonLogger.log("Deleting test item", {
          testItem: testItem.id,
          controllerItems: Array.from(this.controller.items).map(([id, item]) => item.id)
        })

        let { parent } = testItem

        if (parent) {
          parent.children.delete(testItem.id)
        }
        this.controller.items.delete(testItem.id)
      }
    });
  }

  private handleFileChange(event: vscode.TextDocumentChangeEvent) {
    if (!event.document.fileName.endsWith(".swift")) {
      return
    }

    const { path: documentPath } = event.document.uri

    const targetTestItem = this.documentToTargetTestItem.get(documentPath) ?? this.pathToTargetTestItem.find(([path,]) => documentPath.startsWith(path))?.[1]
    if (!targetTestItem) {
      commonLogger.warn("No target test item found for the document", {
        documentPath,
        pathToTargetTestItem: Array.from(this.pathToTargetTestItem.keys()),
        documentToTargetTestItem: Array.from(this.documentToTargetTestItem.keys())
      })
      return
    }
    this.updateTestItems(event.document, targetTestItem)
  }

  async loadTestsFromDefaultScheme() {
    const rootPath = getWorkspacePath();
    const testPlan = parseDefaultTestPlanFile(this.context, rootPath);
    const root = testPlan.defaultOptions.targetForVariableExpansion.name

    const rootTest = this.controller.createTestItem("", root, vscode.Uri.parse(rootPath));

    for (const testTarget of testPlan.testTargets) {
      const { containerPath, name: targetName, identifier } = testTarget.target;
      const [, container] = containerPath.split("container:")

      let fullContainerPath: string
      let testDir: string

      if (container) {
        // test is not a SPM managed package
        if (container.endsWith(".xcodeproj")) {
          fullContainerPath = rootPath
          testDir = path.join(fullContainerPath, targetName)
        } else { // test is a SPM managed package
          fullContainerPath = path.join(rootPath, container)
          testDir = path.join(fullContainerPath, "Tests", identifier)
        }

        const targetTestsParent = this.controller.createTestItem(`${targetName}`, targetName, vscode.Uri.parse(fullContainerPath))

        this.controller.items.add(rootTest)
        rootTest.children.add(targetTestsParent)

        this.pathToTargetTestItem.push([testDir, targetTestsParent])
        let swiftFiles = await findFilesRecursive({
          directory: testDir,
          matcher: (file) => file.isFile() && file.name.endsWith(".swift"),
          depth: 5
        })

        swiftFiles.forEach(fileUri => {
          this.documentToTargetTestItem.set(fileUri, targetTestsParent)
          vscode.workspace.openTextDocument(fileUri).then((file) => {
            this.updateTestItems(file, targetTestsParent)
          })
        })
      }
    }
  }

  alltestsFrom(testItem: vscode.TestItemCollection, items: string[] = []): string[] {
    testItem.forEach((item) => {
      items.push(item.id)
      this.alltestsFrom(item.children, items)
    })

    return items
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
  updateTestItems(document: vscode.TextDocument, parent: vscode.TestItem) {
    // Remove existing test items for this document
    for (const testItem of parent.children) {
      if (testItem[1].uri?.toString() === document.uri.toString()) {
        parent.children.delete(testItem[0]);
      }
    }

    const text = document.getText();

    // Regex to find classes inheriting from XCTestCase that are not commented out
    const classRegex = /^(?!\s*\/\/s*).*[^\S\r\n]*class[^\S\r\n]+(\w+)\s*:\s*XCTestCase\s*\{/gm;
    // let classMatch;
    while (true) {
      const classMatch = classRegex.exec(text);

      if (classMatch === null) {
        break;
      }
      const className = classMatch[1];
      const classPosition = document.positionAt(classMatch.index);

      const classId = `${parent.id}.${className}`

      const classTestItem = this.controller.createTestItem(classId, className, document.uri);
      classTestItem.range = new vscode.Range(classPosition, classPosition);

      parent.children.add(classTestItem);

      const existingTests = this.uriToTestItem.get(document.uri.path) ?? []
      this.uriToTestItem.set(document.uri.path, [...existingTests, classTestItem])

      const classCode = extractCodeBlock(className, text);

      if (classCode === null) {
        continue; // Could not find class code block
      }

      // Find all test methods within the class
      const funcRegex = /^(?!\s*\/\/s*).*func[^\S\r\n]+(test\w+)\s*\(/gm;

      while (true) {
        const funcMatch = funcRegex.exec(classCode);
        if (funcMatch === null) {
          break;
        }
        const testName = funcMatch[1];
        const testStartIndex = classMatch.index + funcMatch.index;
        const position = document.positionAt(testStartIndex);

        const testItem = this.controller.createTestItem(`${classId}.${testName}`, testName, document.uri);
        testItem.range = new vscode.Range(position, position);
        classTestItem.children.add(testItem);
      }
    }
  }

  /**
   * Ask common configuration options for running tests
   */
  async askTestingConfigurations(): Promise<{
    xcworkspace: string;
    scheme: string;
    configuration: string;
    destination: Destination;
  }> {
    // todo: consider to have separate configuration for testing and building. currently we use the
    // configuration for building the project

    const xcworkspace = await askXcodeWorkspacePath(this.context);
    const scheme = await askSchemeForTesting(this.context, {
      xcworkspace: xcworkspace,
      title: "Select a scheme to run tests",
    });
    const configuration = await askConfigurationForTesting(this.context, {
      xcworkspace: xcworkspace,
    });
    const buildSettings = await getBuildSettings({
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });
    const destination = await askDestinationToTestOn(this.context, buildSettings);
    return {
      xcworkspace: xcworkspace,
      scheme: scheme,
      configuration: configuration,
      destination: destination,
    };
  }

  /**
   * Execute separate command to build the project before running tests
   */
  async buildForTestingCommand() {
    const { scheme, destination, xcworkspace } = await this.askTestingConfigurations();

    // before testing we need to build the project to avoid runnning tests on old code or
    // building every time we run selected tests
    await this.buildForTesting({
      destination: destination,
      scheme: scheme,
      xcworkspace: xcworkspace,
    });
  }

  /**
   * Build the project for testing
   */
  async buildForTesting(options: {
    scheme: string;
    destination: Destination;
    xcworkspace: string;
  }) {
    const destinationRaw = getXcodeBuildDestinationString({ destination: options.destination });

    // todo: add xcodebeautify command to format output

    await runTask(this.context, {
      name: "sweetpad.build.build",
      callback: async (terminal) => {
        await terminal.execute({
          command: "xcodebuild",
          args: [
            "build-for-testing",
            "-destination",
            destinationRaw,
            "-allowProvisioningUpdates",
            "-scheme",
            options.scheme,
            "-workspace",
            options.xcworkspace,
          ],
        });
      },
    });
  }

  /**
   * Extract error message from the test output and prepare vscode TestMessage object
   * to display it in the test results.
   */
  getMethodError(options: {
    methodTestId: string;
    runContext: XcodebuildTestRunContext;
  }) {
    const { methodTestId, runContext } = options;

    // Inline error message are usually before the "failed" line
    const error = runContext.getInlineError(methodTestId);
    if (error) {
      // detailed error message with location
      const testMessage = new vscode.TestMessage(error.message);
      testMessage.location = new vscode.Location(
        vscode.Uri.file(error.fileName),
        new vscode.Position(error.lineNumber - 1, 0),
      );
      return testMessage;
    }

    // just geeric error message, no error location or details
    // todo: parse .xcresult file to get more detailed error message
    return new vscode.TestMessage("Test failed (error message is not extracted).");
  }

  getStatusFromOutputLine(
    line: string,
    runContext: XcodebuildTestRunContext,
    testIdSuffix: string
  ): [string?, vscode.TestItem?, string?] {
    const methodStatusMatchIOS = this.METHOD_STATUS_REGEXP_IOS.exec(line);
    const methodStatusMatchMacOS = this.METHOD_STATUS_REGEXP_MACOS.exec(line);
    let status: string = "";
    let methodTest: vscode.TestItem | undefined

    if (methodStatusMatchIOS) {
      const [, className, methodName, lineStatus] = methodStatusMatchIOS;
      const methodTestId = testIdSuffix ? `${testIdSuffix}.${className}.${methodName}` : `${className}.${methodName}`;
      methodTest = runContext.getMethodTest(methodTestId);
      status = lineStatus
    } else if (methodStatusMatchMacOS) {
      // from MacOS we can extract both target and class name
      const [, targetAndclassName, methodName, lineStatus] = methodStatusMatchMacOS;
      const methodTestId = `${targetAndclassName}.${methodName}`

      methodTest = runContext.getMethodTest(methodTestId);
      status = lineStatus
    }

    if (status && methodTest) {
      return [status, methodTest]
    }

    return []
  }

  /**
   * Parse each line of the `xcodebuild` output to update the test run
   * with the test status and any inline error messages.
   */
  async parseOutputLine(options: {
    line: string;
    testIdSuffix: string;
    testRun: vscode.TestRun;
    runContext: XcodebuildTestRunContext;
  }) {
    const { testRun, testIdSuffix, runContext } = options;
    const line = options.line.trim();

    const [status, methodTest] = this.getStatusFromOutputLine(line, runContext, testIdSuffix)
    if (status && methodTest) {
      if (status.startsWith("started")) {
        testRun.started(methodTest);
      } else if (status.startsWith("passed")) {
        testRun.passed(methodTest);
        runContext.addProcessedMethodTest(methodTest.id);
      } else if (status.startsWith("failed")) {
        const error = this.getMethodError({
          methodTestId: methodTest.id,
          runContext: runContext,
        });
        testRun.failed(methodTest, error);
        runContext.addProcessedMethodTest(methodTest.id);
        runContext.addFailedMethodTest(methodTest.id);
      }
      return;
    }

    const inlineErrorMatch = this.INLINE_ERROR_REGEXP.exec(line);
    if (inlineErrorMatch) {
      const [, filePath, lineNumber, targetAndClassName, methodName, errorMessage] = inlineErrorMatch;
      const testId = `${targetAndClassName}.${methodName}`;
      
      runContext.addInlineError(testId, {
        fileName: filePath,
        lineNumber: Number.parseInt(lineNumber, 10),
        message: errorMessage,
      });
    }
  }

  /**
   * Get list of method tests that should be runned
   */
  prepareQueueForRun(request: vscode.TestRunRequest): vscode.TestItem[] {
    const queue: vscode.TestItem[] = [];

    if (request.include) {
      // all tests selected by the user
      queue.push(...request.include);
    } else {
      // all root test items
      queue.push(...[...this.controller.items].map(([, item]) => item));
    }

    // when class test is runned, all its method tests are runned too, so we need to filter out
    // methods that should be runned as part of class test
    return queue
  }

  /**
   * Run selected tests after prepraration and configuration
   */
  async runTests(options: {
    request: vscode.TestRunRequest;
    run: vscode.TestRun;
    xcworkspace: string;
    destination: Destination;
    scheme: string;
    token: vscode.CancellationToken;
  }) {
    const { xcworkspace, scheme, token, run, request } = options;

    const queue = this.prepareQueueForRun(request);

    for (const test of queue) {
      commonLogger.debug("Running single test from queue", {
        testId: test.id,
        testLabel: test.label,
      });

      if (token.isCancellationRequested) {
        run.skipped(test);
        continue;
      }

      const [target, className, methodName] = test.id.split(".");

      if (methodName) {
        await this.runMethodTest({
          run: run,
          methodTest: test,
          xcworkspace: xcworkspace,
          destination: options.destination,
          scheme: scheme,
          target: `${target}/${className}/${methodName}`,
        });
      } else if (className) {
        await this.runClassTest({
          run: run,
          classTest: test,
          scheme: scheme,
          xcworkspace: xcworkspace,
          destination: options.destination,
          target: `${target}/${className}`,
        });
      } else if (target) {
        await this.runTargetTests({
          run: run,
          targetTest: test,
          xcworkspace: xcworkspace,
          destination: options.destination,
          scheme: scheme,
          target: target,
        });
      } else {
        await this.runAllTests({
          run: run,
          root: test,
          xcworkspace: xcworkspace,
          destination: options.destination,
          scheme: scheme,
        })
      }
    }
  }

  /**
   * Run selected tests without building the project
   * This is faster but you may need to build manually before running tests
   */
  async runTestsWithoutBuilding(request: vscode.TestRunRequest, token: vscode.CancellationToken) {
    const run = this.controller.createTestRun(request);
    try {
      const { scheme, destination, xcworkspace } = await this.askTestingConfigurations();

      // todo: add check if project is already built

      await this.runTests({
        run: run,
        request: request,
        xcworkspace: xcworkspace,
        destination: destination,
        scheme: scheme,
        token: token,
      });
    } finally {
      run.end();
    }
  }

  /**
   * Build the project and run the selected tests
   */
  async buildAndRunTests(request: vscode.TestRunRequest, token: vscode.CancellationToken) {
    const run = this.controller.createTestRun(request);
    try {
      const { scheme, destination, xcworkspace } = await this.askTestingConfigurations();

      // before testing we need to build the project to avoid runnning tests on old code or
      // building every time we run selected tests
      await this.buildForTesting({
        scheme: scheme,
        destination: destination,
        xcworkspace: xcworkspace,
      });

      commonLogger.log("Project is built, running tests", {});

      await this.runTests({
        run: run,
        request: request,
        xcworkspace: xcworkspace,
        destination: destination,
        scheme: scheme,
        token: token,
      });

      commonLogger.log("Tests ended", {});
    } finally {
      run.end();
    }
  }

  async runClassTest(options: {
    run: vscode.TestRun;
    classTest: vscode.TestItem;
    scheme: string;
    xcworkspace: string;
    destination: Destination;
    target: string;
  }): Promise<void> {
    const { run, classTest, scheme, target } = options;
    const [targetName, ,] = classTest.id.split(".");

    const runContext = new XcodebuildTestRunContext({
      methodTests: [...classTest.children],
    });

    const destinationRaw = getXcodeBuildDestinationString({ destination: options.destination });

    classTest.children.forEach((methodTest) => {
      run.started(methodTest);
    });

    try {
      await runTask(this.context, {
        name: "sweetpad.testing.test",
        callback: async (terminal) => {
          await terminal.execute({
            command: "xcodebuild",
            args: [
              "test-without-building",
              "-workspace",
              options.xcworkspace,
              "-destination",
              destinationRaw,
              "-scheme",
              scheme,
              `-only-testing:${target}`,
            ],
            onOutputLine: async (output) => {
              console.log("output", output);
              await this.parseOutputLine({
                line: output.value,
                testRun: run,
                testIdSuffix: targetName,
                runContext: runContext,
              });
            },
          });
        },
      });
    } catch (error) {
      console.error("Test class failed due to an error", error);
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
    }
  }

  async runMethodTest(options: {
    run: vscode.TestRun;
    methodTest: vscode.TestItem;
    xcworkspace: string;
    scheme: string;
    destination: Destination;
    target: string;
  }): Promise<void> {
    const { run: testRun, methodTest, scheme, target } = options;
    const [targetName,] = methodTest.id.split(".");

    const runContext = new XcodebuildTestRunContext({
      methodTests: [[methodTest.id, methodTest]],
    });

    const destinationRaw = getXcodeBuildDestinationString({ destination: options.destination });

    // Run "xcodebuild" command as a task to see the test output
    await runTask(this.context, {
      name: "sweetpad.testing.test",
      callback: async (terminal) => {
        try {
          await terminal.execute({
            command: "xcodebuild",
            args: [
              "test-without-building",
              "-workspace",
              options.xcworkspace,
              "-destination",
              destinationRaw,
              "-scheme",
              scheme,
              `-only-testing:${target}`,
            ],
            onOutputLine: async (output) => {
              await this.parseOutputLine({
                line: output.value,
                testRun: testRun,
                testIdSuffix: targetName,
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

  async runTargetTests(options: {
    run: vscode.TestRun;
    targetTest: vscode.TestItem;
    scheme: string;
    xcworkspace: string;
    destination: Destination;
    target: string;
  }): Promise<void> {
    const { run, targetTest, scheme, target } = options;
    const [targetName, ,] = targetTest.id.split(".");

    let methodTests: Iterable<[string, vscode.TestItem]> = [];

    targetTest.children.forEach((classTest) => {
      methodTests = [...methodTests, ...classTest.children]
    })

    const runContext = new XcodebuildTestRunContext({
      methodTests,
    });

    const destinationRaw = getXcodeBuildDestinationString({ destination: options.destination });

    targetTest.children.forEach((classTest) => {
      classTest.children.forEach((methodTest) => {
        run.started(methodTest);
      })
    })

    try {
      await runTask(this.context, {
        name: "sweetpad.testing.test",
        callback: async (terminal) => {
          await terminal.execute({
            command: "xcodebuild",
            args: [
              "test-without-building",
              "-workspace",
              options.xcworkspace,
              "-destination",
              destinationRaw,
              "-scheme",
              scheme,
              `-only-testing:${target}`,
            ],
            onOutputLine: async (output) => {
              console.log("output", output);
              await this.parseOutputLine({
                line: output.value,
                testRun: run,
                testIdSuffix: targetName,
                runContext: runContext,
              });
            },
          });
        },
      });
    } catch (error) {
      console.error("Test class failed due to an error", error);
      // Handle any errors during test execution
      const errorMessage = `Test class failed due to an error: ${error instanceof Error ? error.message : "Test failed"}`;
      run.failed(targetTest, new vscode.TestMessage(errorMessage));

      // Mark all unprocessed child tests as failed
      for (const methodTest of runContext.getUnprocessedMethodTests()) {
        run.failed(methodTest, new vscode.TestMessage("Test failed due to an error."));
      }
    } finally {
      // Mark any unprocessed tests as skipped
      for (const methodTest of runContext.getUnprocessedMethodTests()) {
        run.skipped(methodTest);
      }
    }
  }

  async runAllTests(options: {
    run: vscode.TestRun;
    root: vscode.TestItem;
    xcworkspace: string;
    scheme: string;
    destination: Destination;
  }) {
    const { run, root, scheme } = options;

    let methodTests: Iterable<[string, vscode.TestItem]> = [];

    root.children.forEach((targetTests) => {
      targetTests.children.forEach((classTest) => {
        methodTests = [...methodTests, ...classTest.children]
      })
    })

    const runContext = new XcodebuildTestRunContext({
      methodTests
    });

    const destinationRaw = getXcodeBuildDestinationString({ destination: options.destination });

    root.children.forEach((targetTests) => {
      targetTests.children.forEach((classTest) => {
        classTest.children.forEach((methodTest) => {
          run.started(methodTest);
        })
      });
    })

    try {
      await runTask(this.context, {
        name: "sweetpad.testing.test",
        callback: async (terminal) => {
          await terminal.execute({
            command: "xcodebuild",
            args: [
              "test-without-building",
              "-workspace",
              options.xcworkspace,
              "-destination",
              destinationRaw,
              "-scheme",
              scheme
            ],
            onOutputLine: async (output) => {
              console.log("output", output);
              await this.parseOutputLine({
                line: output.value,
                testRun: run,
                testIdSuffix: "",
                runContext: runContext,
              });
            },
          });
        },
      });
    } catch (error) {
      console.error("Test class failed due to an error", error);
      // Handle any errors during test execution
      const errorMessage = `Test class failed due to an error: ${error instanceof Error ? error.message : "Test failed"}`;
      run.failed(root, new vscode.TestMessage(errorMessage));

      // Mark all unprocessed child tests as failed
      for (const methodTest of runContext.getUnprocessedMethodTests()) {
        run.failed(methodTest, new vscode.TestMessage("Test failed due to an error."));
      }
    } finally {
      // Mark any unprocessed tests as skipped
      for (const methodTest of runContext.getUnprocessedMethodTests()) {
        run.skipped(methodTest);
      }
    }
  }
}
