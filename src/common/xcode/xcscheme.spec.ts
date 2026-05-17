import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";

import {
  BuildAction,
  BuildActionEntry,
  BuildableProductRunnable,
  BuildableReference,
  EnvironmentVariable,
  LaunchAction,
  SchemeDocument,
  XcSchemeParseError,
} from "./xcscheme";

const FIXTURE_DIR = path.resolve(__dirname, "../../../tests/xcscheme-data");
const FIXTURES = readdirSync(FIXTURE_DIR)
  .filter((f) => f.endsWith(".xcscheme"))
  .toSorted();

function loadFixture(name: string): string {
  return readFileSync(path.join(FIXTURE_DIR, name), "utf8");
}

function wrapScheme(inner: string): string {
  return `<?xml version="1.0" encoding="UTF-8"?>\n<Scheme\n   LastUpgradeVersion = "1500"\n   version = "1.7">\n${inner}\n</Scheme>\n`;
}

describe("SchemeDocument.parse — strict on top-level", () => {
  it("throws XcSchemeParseError on malformed XML", () => {
    expect(() => SchemeDocument.parse("<<not xml>>")).toThrow(XcSchemeParseError);
    expect(() => SchemeDocument.parse("<<not xml>>")).toThrow(/Invalid xcscheme XML/);
  });

  it("throws when root is not <Scheme>", () => {
    expect(() => SchemeDocument.parse('<?xml version="1.0"?>\n<NotAScheme/>')).toThrow(/root element must be <Scheme>/);
  });

  it("includes line/column context on parse failure", () => {
    let caught: XcSchemeParseError | undefined;
    try {
      SchemeDocument.parse('<?xml version="1.0"?>\n<Scheme>\n   <BadlyClosed>\n</Scheme>');
    } catch (err) {
      caught = err as XcSchemeParseError;
    }
    expect(caught).toBeInstanceOf(XcSchemeParseError);
    expect(caught?.line).toBeGreaterThan(0);
  });

  it("accepts a minimal <Scheme> root with no children", () => {
    const doc = SchemeDocument.parse('<?xml version="1.0" encoding="UTF-8"?>\n<Scheme version="1.7"/>');
    expect(doc.version).toBe("1.7");
    expect(doc.buildAction()).toBeUndefined();
  });
});

describe("SchemeDocument — schema version + XML declaration", () => {
  it("stamps the parsed document with the schema version", () => {
    const doc = SchemeDocument.parse(loadFixture("wikipedia-ios-rtl.xcscheme"));
    expect(doc.schemaVersion).toBe("1");
  });

  it("captures the XML declaration from the source", () => {
    const doc = SchemeDocument.parse(loadFixture("wikipedia-ios-rtl.xcscheme"));
    expect(doc.xmlDeclaration).toEqual({ version: "1.0", encoding: "UTF-8" });
  });

  it("preserves a custom standalone declaration through round-trip", () => {
    const xml = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<Scheme>\n</Scheme>\n';
    const doc = SchemeDocument.parse(xml);
    expect(doc.xmlDeclaration.standalone).toBe("yes");
    expect(doc.serialize()).toBe(xml);
  });

  it("defaults declaration to version 1.0 / encoding UTF-8 for fresh documents", () => {
    const doc = new SchemeDocument();
    expect(doc.xmlDeclaration).toEqual({ version: "1.0", encoding: "UTF-8" });
  });
});

describe("attribute handling", () => {
  it("coerces YES/NO to boolean for known boolean attributes", () => {
    const doc = SchemeDocument.parse(
      wrapScheme(
        `   <BuildAction\n      parallelizeBuildables = "YES"\n      buildImplicitDependencies = "NO">\n   </BuildAction>`,
      ),
    );
    expect(doc.buildAction()?.parallelizeBuildables).toBe(true);
    expect(doc.buildAction()?.buildImplicitDependencies).toBe(false);
  });

  it("preserves attribute order across round-trip via insertion-ordered Map", () => {
    const doc = SchemeDocument.parse(
      wrapScheme(
        `   <LaunchAction\n      language = "he"\n      buildConfiguration = "Debug"\n      region = "IL">\n   </LaunchAction>`,
      ),
    );
    const entries = doc.launchAction()?.attributeEntries() ?? [];
    expect(entries.map(([k]) => k)).toEqual(["language", "buildConfiguration", "region"]);
  });

  it("setting an attribute to undefined removes it", () => {
    const launch = new LaunchAction();
    launch.language = "ar";
    expect(launch.language).toBe("ar");
    launch.language = undefined;
    expect(launch.language).toBeUndefined();
    expect(launch.hasAttribute("language")).toBe(false);
  });

  it("re-setting an existing attribute updates in place, preserving order", () => {
    const doc = SchemeDocument.parse(
      wrapScheme(`   <LaunchAction\n      buildConfiguration = "Debug"\n      language = "he">\n   </LaunchAction>`),
    );
    const launch = doc.launchAction()!;
    launch.language = "ar";
    const entries = launch.attributeEntries();
    expect(entries).toEqual([
      ["buildConfiguration", "Debug"],
      ["language", "ar"],
    ]);
  });
});

describe("LaunchAction — discussion #197 use case", () => {
  it("extracts language, region, command-line args, and env vars from Wikipedia RTL fixture", () => {
    const doc = SchemeDocument.parse(loadFixture("wikipedia-ios-rtl.xcscheme"));
    const launch = doc.launchAction()!;
    expect(launch.language).toBe("he");
    expect(launch.region).toBe("IL");
    const args = launch.commandLineArguments();
    expect(args).toHaveLength(3);
    expect(args[0].argument).toBe("-AppleLocale he_IL");
    expect(args[0].isEnabled).toBe(true);
    expect(args[1].argument).toBe("-AppleLanguages (he)");
  });

  it("recognises disabled command-line arguments (isEnabled = NO)", () => {
    const doc = SchemeDocument.parse(loadFixture("duckduckgo-ios-browser.xcscheme"));
    const args = doc.launchAction()?.commandLineArguments() ?? [];
    const sqlDebug = args.find((a) => a.argument === "-com.apple.CoreData.SQLDebug 1");
    expect(sqlDebug?.isEnabled).toBe(false);
  });

  it("parses EnvironmentVariables with key/value/isEnabled", () => {
    const doc = SchemeDocument.parse(loadFixture("duckduckgo-ios-browser.xcscheme"));
    const envs = doc.launchAction()?.environmentVariables() ?? [];
    expect(envs.map((e) => ({ key: e.key, value: e.value, isEnabled: e.isEnabled }))).toEqual([
      { key: "ONBOARDING", value: "true", isEnabled: false },
      { key: "VARIANT", value: "ma", isEnabled: false },
    ]);
  });

  it("parses LocationScenarioReference", () => {
    const doc = SchemeDocument.parse(loadFixture("duckduckgo-ios-browser.xcscheme"));
    const loc = doc.launchAction()?.locationScenarioReference();
    expect(loc?.identifier).toBe("London, England");
    expect(loc?.referenceType).toBe("1");
  });

  it("reads BuildableProductRunnable → BuildableReference → blueprintName", () => {
    const doc = SchemeDocument.parse(loadFixture("wikipedia-ios.xcscheme"));
    const ref = doc.launchAction()?.buildableProductRunnable()?.buildableReference();
    expect(ref?.blueprintName).toBe("Wikipedia");
    expect(ref?.buildableName).toBe("Wikipedia.app");
  });
});

describe("LaunchAction — mutation API", () => {
  it("setAppLocale sets language and region together", () => {
    const launch = new LaunchAction();
    launch.setAppLocale("ar", "SA");
    expect(launch.language).toBe("ar");
    expect(launch.region).toBe("SA");
  });

  it("addCommandLineArgument creates the container on demand and appends", () => {
    const launch = new LaunchAction();
    launch.addCommandLineArgument({ argument: "--verbose" });
    launch.addCommandLineArgument({ argument: "--debug", isEnabled: false });
    const args = launch.commandLineArguments();
    expect(args).toHaveLength(2);
    expect(args[0].argument).toBe("--verbose");
    expect(args[0].isEnabled).toBe(true);
    expect(args[1].isEnabled).toBe(false);
  });

  it("clearCommandLineArguments removes the entire container", () => {
    const launch = new LaunchAction();
    launch.addCommandLineArgument({ argument: "--a" });
    launch.clearCommandLineArguments();
    expect(launch.commandLineArguments()).toEqual([]);
  });

  it("addEnvironmentVariable accepts both class instances and plain init objects", () => {
    const launch = new LaunchAction();
    launch.addEnvironmentVariable({ key: "FROM_OBJECT", value: "1" });
    const explicit = new EnvironmentVariable();
    explicit.key = "FROM_INSTANCE";
    explicit.value = "2";
    explicit.isEnabled = false;
    launch.addEnvironmentVariable(explicit);
    const envs = launch.environmentVariables();
    expect(envs).toHaveLength(2);
    expect(envs[0].key).toBe("FROM_OBJECT");
    expect(envs[1].isEnabled).toBe(false);
  });
});

describe("TestAction", () => {
  it("parses sanitizers as booleans", () => {
    const doc = SchemeDocument.parse(loadFixture("parse-sdk-ios.xcscheme"));
    const test = doc.testAction()!;
    expect(test.enableAddressSanitizer).toBe(true);
    expect(test.enableASanStackUseAfterReturn).toBe(true);
    expect(test.enableUBSanitizer).toBe(true);
    expect(test.codeCoverageEnabled).toBe(true);
  });

  it("parses TestableReference with SkippedTests", () => {
    const doc = SchemeDocument.parse(loadFixture("parse-sdk-ios.xcscheme"));
    const testable = doc.testAction()?.testables()[0]!;
    expect(testable.buildableReference()?.blueprintName).toBe("ParseUnitTests-iOS");
    const skipped = testable.skippedTests();
    expect(skipped.length).toBeGreaterThan(0);
    expect(skipped[0].identifier).toBe("ExtensionDataSharingMobileTests");
  });

  it("parses TestPlans → TestPlanReference", () => {
    const doc = SchemeDocument.parse(loadFixture("alamofire-ios.xcscheme"));
    const tp = doc.testAction()?.testPlans()[0];
    expect(tp?.reference).toBe("container:Tests/Test Plans/iOS.xctestplan");
    expect(tp?.isDefault).toBe(true);
  });

  it("parses AdditionalOptions", () => {
    const doc = SchemeDocument.parse(loadFixture("alamofire-ios.xcscheme"));
    const opt = doc.testAction()?.additionalOptions()[0];
    expect(opt?.key).toBe("NSZombieEnabled");
    expect(opt?.value).toBe("YES");
    expect(opt?.isEnabled).toBe(true);
  });
});

describe("PreActions / PostActions with shell scripts", () => {
  it("parses ExecutionAction → ActionContent, decoding entity-encoded scriptText", () => {
    const doc = SchemeDocument.parse(loadFixture("realm-swift.xcscheme"));
    const exec = doc.buildAction()?.preActions()[0]!;
    expect(exec.actionType).toBe("Xcode.IDEStandardExecutionActionsCore.ExecutionActionType.ShellScriptAction");
    const content = exec.actionContent()!;
    expect(content.title).toBe("Run Script");
    expect(content.shellToInvoke).toBe("/bin/sh");
    expect(content.scriptText).toBe('cd "${PROJECT_DIR}"\nsh build.sh download-core\n');
    expect(content.environmentBuildable()?.buildableReference()?.blueprintName).toBe("Realm");
  });
});

describe("targetToLaunch convenience", () => {
  it("returns the BlueprintName of the launchable target", () => {
    const doc = SchemeDocument.parse(loadFixture("wikipedia-ios-rtl.xcscheme"));
    expect(doc.targetToLaunch()).toBe("Wikipedia");
  });

  it("returns null when there's no LaunchAction", () => {
    const doc = SchemeDocument.parse('<?xml version="1.0"?>\n<Scheme version="1.7"/>');
    expect(doc.targetToLaunch()).toBeNull();
  });

  it("returns null for framework schemes (LaunchAction has only a MacroExpansion)", () => {
    const doc = SchemeDocument.parse(loadFixture("alamofire-ios.xcscheme"));
    expect(doc.launchAction()?.buildableProductRunnable()).toBeUndefined();
    expect(doc.targetToLaunch()).toBeNull();
  });
});

describe("comments preservation", () => {
  it("preserves comments through round-trip", () => {
    const xml = wrapScheme(
      `   <!-- top-level note -->\n   <BuildAction>\n      <!-- inner note -->\n   </BuildAction>`,
    );
    const doc = SchemeDocument.parse(xml);
    expect(doc.serialize()).toBe(xml);
  });

  it("comments() reports text with positional hints", () => {
    const xml = wrapScheme(
      `   <BuildAction>\n   </BuildAction>\n   <!-- between build and test -->\n   <TestAction>\n   </TestAction>`,
    );
    const doc = SchemeDocument.parse(xml);
    const cs = doc.comments();
    expect(cs).toHaveLength(1);
    expect(cs[0].text).toBe(" between build and test ");
    expect(cs[0].precededBy).toBe("BuildAction");
    expect(cs[0].followedBy).toBe("TestAction");
  });

  it("addCommentBefore inserts a new comment at the right slot", () => {
    const doc = new SchemeDocument();
    doc.setBuildAction(new BuildAction());
    doc.setLaunchAction(new LaunchAction());
    doc.addCommentBefore("LaunchAction", " RTL test build ");
    expect(doc.comments()).toEqual([expect.objectContaining({ text: " RTL test build ", followedBy: "LaunchAction" })]);
  });

  it("removeComments(predicate) drops matching comments", () => {
    const xml = wrapScheme(`   <!-- keep this -->\n   <!-- TODO drop this -->\n   <BuildAction>\n   </BuildAction>`);
    const doc = SchemeDocument.parse(xml);
    doc.removeComments((t) => /TODO/.test(t));
    const remaining = doc.comments();
    expect(remaining).toHaveLength(1);
    expect(remaining[0].text).toBe(" keep this ");
  });
});

describe("CDATA preservation", () => {
  it("preserves CDATA sections through round-trip", () => {
    const xml = wrapScheme(`   <BuildAction>\n      <![CDATA[some <raw> & literal text]]>\n   </BuildAction>`);
    const doc = SchemeDocument.parse(xml);
    expect(doc.serialize()).toBe(xml);
  });
});

describe("extras passthrough", () => {
  it("preserves unknown child elements via the extras slot", () => {
    const xml = wrapScheme(
      `   <BuildAction>\n      <FutureElement\n         attr = "x">\n      </FutureElement>\n   </BuildAction>`,
    );
    const doc = SchemeDocument.parse(xml);
    expect(doc.serialize()).toBe(xml);
    expect(doc.buildAction()?.extraChildren()).toHaveLength(1);
    expect(doc.buildAction()?.extraChildren()[0].name).toBe("FutureElement");
  });
});

describe("clone — deep copy for undo / immutable updates", () => {
  it("clone produces an independent tree", () => {
    const doc = SchemeDocument.parse(loadFixture("wikipedia-ios-rtl.xcscheme"));
    const copy = doc.clone();
    expect(copy).not.toBe(doc);
    expect(copy.launchAction()).not.toBe(doc.launchAction());
    copy.launchAction()!.language = "ar";
    expect(doc.launchAction()?.language).toBe("he");
    expect(copy.launchAction()?.language).toBe("ar");
  });
});

describe("byte-identical round-trip across real-world fixtures", () => {
  it.each(FIXTURES)("%s", (name) => {
    const xml = loadFixture(name);
    const doc = SchemeDocument.parse(xml);
    expect(doc.serialize()).toBe(xml);
  });
});

/**
 * Focused parse → serialize tests, one row per XML shape. Each parses the
 * snippet, then asserts the serialized output is byte-identical to the input.
 * These act both as regression tests for the per-element parser/serializer
 * and as living documentation of the exact wire format the module emits.
 */
const ROUND_TRIP_CASES: ReadonlyArray<[label: string, xml: string]> = [
  // --- Document / declaration ---------------------------------------------
  ["minimal <Scheme> with no attrs or children", '<?xml version="1.0" encoding="UTF-8"?>\n<Scheme>\n</Scheme>\n'],
  [
    "<Scheme> with two attributes",
    '<?xml version="1.0" encoding="UTF-8"?>\n<Scheme\n   LastUpgradeVersion = "1500"\n   version = "1.7">\n</Scheme>\n',
  ],
  [
    "XML declaration with standalone=yes",
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<Scheme>\n</Scheme>\n',
  ],
  ["XML declaration with no encoding", '<?xml version="1.0"?>\n<Scheme>\n</Scheme>\n'],

  // --- BuildAction --------------------------------------------------------
  [
    "<BuildAction> with no children",
    wrapScheme(
      `   <BuildAction\n      parallelizeBuildables = "YES"\n      buildImplicitDependencies = "YES">\n   </BuildAction>`,
    ),
  ],
  [
    "<BuildAction> with <BuildActionEntries> containing one <BuildActionEntry>",
    wrapScheme(
      `   <BuildAction\n      parallelizeBuildables = "YES"\n      buildImplicitDependencies = "YES">\n      <BuildActionEntries>\n         <BuildActionEntry\n            buildForTesting = "YES"\n            buildForRunning = "YES"\n            buildForProfiling = "YES"\n            buildForArchiving = "YES"\n            buildForAnalyzing = "YES">\n            <BuildableReference\n               BuildableIdentifier = "primary"\n               BlueprintIdentifier = "A1B2C3D4"\n               BuildableName = "MyApp.app"\n               BlueprintName = "MyApp"\n               ReferencedContainer = "container:MyApp.xcodeproj">\n            </BuildableReference>\n         </BuildActionEntry>\n      </BuildActionEntries>\n   </BuildAction>`,
    ),
  ],
  [
    "<BuildAction> with multiple <BuildActionEntry>",
    wrapScheme(
      `   <BuildAction\n      parallelizeBuildables = "YES"\n      buildImplicitDependencies = "YES">\n      <BuildActionEntries>\n         <BuildActionEntry\n            buildForTesting = "YES"\n            buildForRunning = "YES"\n            buildForProfiling = "YES"\n            buildForArchiving = "YES"\n            buildForAnalyzing = "YES">\n            <BuildableReference\n               BuildableIdentifier = "primary"\n               BlueprintName = "MyApp">\n            </BuildableReference>\n         </BuildActionEntry>\n         <BuildActionEntry\n            buildForTesting = "YES"\n            buildForRunning = "NO"\n            buildForProfiling = "NO"\n            buildForArchiving = "NO"\n            buildForAnalyzing = "NO">\n            <BuildableReference\n               BuildableIdentifier = "primary"\n               BlueprintName = "MyAppTests">\n            </BuildableReference>\n         </BuildActionEntry>\n      </BuildActionEntries>\n   </BuildAction>`,
    ),
  ],

  // --- TestAction ---------------------------------------------------------
  [
    "<TestAction> with sanitizers and code coverage flags",
    wrapScheme(
      `   <TestAction\n      buildConfiguration = "Debug"\n      selectedDebuggerIdentifier = "Xcode.DebuggerFoundation.Debugger.LLDB"\n      selectedLauncherIdentifier = "Xcode.DebuggerFoundation.Launcher.LLDB"\n      shouldUseLaunchSchemeArgsEnv = "YES"\n      enableAddressSanitizer = "YES"\n      enableASanStackUseAfterReturn = "YES"\n      enableUBSanitizer = "YES"\n      codeCoverageEnabled = "YES">\n   </TestAction>`,
    ),
  ],
  [
    "<TestAction> with <Testables> + <TestableReference> + <SkippedTests>",
    wrapScheme(
      `   <TestAction\n      buildConfiguration = "Debug">\n      <Testables>\n         <TestableReference\n            skipped = "NO">\n            <BuildableReference\n               BuildableIdentifier = "primary"\n               BlueprintName = "MyAppTests">\n            </BuildableReference>\n            <SkippedTests>\n               <Test\n                  Identifier = "FlakyTest1">\n               </Test>\n               <Test\n                  Identifier = "FlakyTest2">\n               </Test>\n            </SkippedTests>\n         </TestableReference>\n      </Testables>\n   </TestAction>`,
    ),
  ],
  [
    "<TestAction> with <TestPlans> + <TestPlanReference>",
    wrapScheme(
      `   <TestAction\n      buildConfiguration = "Debug">\n      <TestPlans>\n         <TestPlanReference\n            reference = "container:Tests/Plan.xctestplan"\n            default = "YES">\n         </TestPlanReference>\n      </TestPlans>\n   </TestAction>`,
    ),
  ],
  [
    "<TestAction> with <AdditionalOptions>",
    wrapScheme(
      `   <TestAction\n      buildConfiguration = "Debug">\n      <AdditionalOptions>\n         <AdditionalOption\n            key = "NSZombieEnabled"\n            value = "YES"\n            isEnabled = "YES">\n         </AdditionalOption>\n      </AdditionalOptions>\n   </TestAction>`,
    ),
  ],
  [
    "<TestAction> with <CodeCoverageTargets>",
    wrapScheme(
      `   <TestAction\n      buildConfiguration = "Debug"\n      codeCoverageEnabled = "YES"\n      onlyGenerateCoverageForSpecifiedTargets = "YES">\n      <CodeCoverageTargets>\n         <BuildableReference\n            BuildableIdentifier = "primary"\n            BlueprintName = "MyApp">\n         </BuildableReference>\n      </CodeCoverageTargets>\n   </TestAction>`,
    ),
  ],

  // --- LaunchAction -------------------------------------------------------
  [
    "<LaunchAction> minimal",
    wrapScheme(
      `   <LaunchAction\n      buildConfiguration = "Debug"\n      selectedDebuggerIdentifier = "Xcode.DebuggerFoundation.Debugger.LLDB"\n      selectedLauncherIdentifier = "Xcode.DebuggerFoundation.Launcher.LLDB"\n      launchStyle = "0"\n      useCustomWorkingDirectory = "NO"\n      ignoresPersistentStateOnLaunch = "NO"\n      debugDocumentVersioning = "YES"\n      debugServiceExtension = "internal"\n      allowLocationSimulation = "YES">\n   </LaunchAction>`,
    ),
  ],
  [
    "<LaunchAction> with language + region (discussion #197)",
    wrapScheme(
      `   <LaunchAction\n      buildConfiguration = "Debug"\n      language = "ar"\n      region = "SA"\n      allowLocationSimulation = "YES">\n   </LaunchAction>`,
    ),
  ],
  [
    "<LaunchAction> with <BuildableProductRunnable> + <BuildableReference>",
    wrapScheme(
      `   <LaunchAction\n      buildConfiguration = "Debug">\n      <BuildableProductRunnable\n         runnableDebuggingMode = "0">\n         <BuildableReference\n            BuildableIdentifier = "primary"\n            BlueprintIdentifier = "A1B2C3"\n            BuildableName = "MyApp.app"\n            BlueprintName = "MyApp"\n            ReferencedContainer = "container:MyApp.xcodeproj">\n         </BuildableReference>\n      </BuildableProductRunnable>\n   </LaunchAction>`,
    ),
  ],
  [
    "<LaunchAction> with <RemoteRunnable>",
    wrapScheme(
      `   <LaunchAction\n      buildConfiguration = "Debug">\n      <RemoteRunnable\n         runnableDebuggingMode = "2"\n         BundleIdentifier = "com.example.myapp"\n         RemotePath = "/Applications/MyApp.app">\n         <BuildableReference\n            BuildableIdentifier = "primary"\n            BlueprintName = "MyApp">\n         </BuildableReference>\n      </RemoteRunnable>\n   </LaunchAction>`,
    ),
  ],
  [
    "<LaunchAction> with <MacroExpansion> (framework scheme)",
    wrapScheme(
      `   <LaunchAction\n      buildConfiguration = "Debug">\n      <MacroExpansion>\n         <BuildableReference\n            BuildableIdentifier = "primary"\n            BlueprintName = "MyFramework"\n            BuildableName = "MyFramework.framework">\n         </BuildableReference>\n      </MacroExpansion>\n   </LaunchAction>`,
    ),
  ],
  [
    "<LaunchAction> with <CommandLineArguments> mixing enabled and disabled",
    wrapScheme(
      `   <LaunchAction\n      buildConfiguration = "Debug">\n      <CommandLineArguments>\n         <CommandLineArgument\n            argument = "--verbose"\n            isEnabled = "YES">\n         </CommandLineArgument>\n         <CommandLineArgument\n            argument = "--debug"\n            isEnabled = "NO">\n         </CommandLineArgument>\n      </CommandLineArguments>\n   </LaunchAction>`,
    ),
  ],
  [
    "<LaunchAction> with <EnvironmentVariables>",
    wrapScheme(
      `   <LaunchAction\n      buildConfiguration = "Debug">\n      <EnvironmentVariables>\n         <EnvironmentVariable\n            key = "DEBUG"\n            value = "1"\n            isEnabled = "YES">\n         </EnvironmentVariable>\n         <EnvironmentVariable\n            key = "FEATURE_FLAG"\n            value = "off"\n            isEnabled = "NO">\n         </EnvironmentVariable>\n      </EnvironmentVariables>\n   </LaunchAction>`,
    ),
  ],
  [
    "<LaunchAction> with <LocationScenarioReference>",
    wrapScheme(
      `   <LaunchAction\n      buildConfiguration = "Debug"\n      allowLocationSimulation = "YES">\n      <LocationScenarioReference\n         identifier = "London, England"\n         referenceType = "1">\n      </LocationScenarioReference>\n   </LaunchAction>`,
    ),
  ],
  [
    "<LaunchAction> with sanitizers and GPU validation modes",
    wrapScheme(
      `   <LaunchAction\n      buildConfiguration = "Debug"\n      enableAddressSanitizer = "YES"\n      enableThreadSanitizer = "YES"\n      stopOnEveryUBSanitizerIssue = "YES"\n      enableGPUValidationMode = "1"\n      enableGPUFrameCaptureMode = "0">\n   </LaunchAction>`,
    ),
  ],

  // --- ProfileAction / AnalyzeAction / ArchiveAction ----------------------
  [
    "<ProfileAction> with MacroExpansion (framework profiling)",
    wrapScheme(
      `   <ProfileAction\n      buildConfiguration = "Release"\n      shouldUseLaunchSchemeArgsEnv = "YES"\n      savedToolIdentifier = ""\n      useCustomWorkingDirectory = "NO"\n      debugDocumentVersioning = "YES">\n      <MacroExpansion>\n         <BuildableReference\n            BuildableIdentifier = "primary"\n            BlueprintName = "MyFramework">\n         </BuildableReference>\n      </MacroExpansion>\n   </ProfileAction>`,
    ),
  ],
  [
    "<AnalyzeAction> minimal",
    wrapScheme(`   <AnalyzeAction\n      buildConfiguration = "Debug">\n   </AnalyzeAction>`),
  ],
  [
    "<ArchiveAction> with custom archive name",
    wrapScheme(
      `   <ArchiveAction\n      buildConfiguration = "Release"\n      revealArchiveInOrganizer = "YES"\n      customArchiveName = "Release-2026-05-17">\n   </ArchiveAction>`,
    ),
  ],

  // --- PreActions / PostActions -------------------------------------------
  [
    "<BuildAction> with a <PreActions> shell-script <ExecutionAction>",
    wrapScheme(
      `   <BuildAction\n      parallelizeBuildables = "YES"\n      buildImplicitDependencies = "YES">\n      <PreActions>\n         <ExecutionAction\n            ActionType = "Xcode.IDEStandardExecutionActionsCore.ExecutionActionType.ShellScriptAction">\n            <ActionContent\n               title = "Run Script"\n               scriptText = "echo before-build"\n               shellToInvoke = "/bin/sh">\n               <EnvironmentBuildable>\n                  <BuildableReference\n                     BuildableIdentifier = "primary"\n                     BlueprintName = "MyApp">\n                  </BuildableReference>\n               </EnvironmentBuildable>\n            </ActionContent>\n         </ExecutionAction>\n      </PreActions>\n   </BuildAction>`,
    ),
  ],
  [
    "<BuildAction> with <PostActions> + runPostActionsOnFailure",
    wrapScheme(
      `   <BuildAction\n      parallelizeBuildables = "YES"\n      buildImplicitDependencies = "YES"\n      runPostActionsOnFailure = "YES">\n      <PostActions>\n         <ExecutionAction\n            ActionType = "Xcode.IDEStandardExecutionActionsCore.ExecutionActionType.ShellScriptAction">\n            <ActionContent\n               title = "Cleanup"\n               scriptText = "rm -rf /tmp/build-artifacts"\n               shellToInvoke = "/bin/sh">\n            </ActionContent>\n         </ExecutionAction>\n      </PostActions>\n   </BuildAction>`,
    ),
  ],
  [
    "shell-script with XML-escaped quotes and newlines",
    wrapScheme(
      `   <BuildAction>\n      <PreActions>\n         <ExecutionAction\n            ActionType = "Xcode.IDEStandardExecutionActionsCore.ExecutionActionType.ShellScriptAction">\n            <ActionContent\n               title = "Multi-line"\n               scriptText = "cd &quot;\${PROJECT_DIR}&quot;&#10;echo done&#10;"\n               shellToInvoke = "/bin/sh">\n            </ActionContent>\n         </ExecutionAction>\n      </PreActions>\n   </BuildAction>`,
    ),
  ],

  // --- Comments / CDATA ---------------------------------------------------
  [
    "comment at the top level, between actions",
    wrapScheme(
      `   <BuildAction>\n   </BuildAction>\n   <!-- this is a top-level comment between actions -->\n   <TestAction\n      buildConfiguration = "Debug">\n   </TestAction>`,
    ),
  ],
  [
    "comment inside an element, between child slots",
    wrapScheme(
      `   <BuildAction>\n      <!-- entries explanation -->\n      <BuildActionEntries>\n      </BuildActionEntries>\n   </BuildAction>`,
    ),
  ],
  [
    "CDATA section inside an element",
    wrapScheme(`   <BuildAction>\n      <![CDATA[anything <weird> goes here & it's preserved]]>\n   </BuildAction>`),
  ],

  // --- Unknown elements ---------------------------------------------------
  [
    "unknown top-level child preserved as extras",
    wrapScheme(`   <BuildAction>\n   </BuildAction>\n   <FutureAction\n      apiKey = "v2">\n   </FutureAction>`),
  ],
  [
    "unknown child inside a known element preserved as extras",
    wrapScheme(
      `   <LaunchAction\n      buildConfiguration = "Debug">\n      <BuildableProductRunnable\n         runnableDebuggingMode = "0">\n         <BuildableReference\n            BuildableIdentifier = "primary"\n            BlueprintName = "MyApp">\n         </BuildableReference>\n      </BuildableProductRunnable>\n      <FutureSetting\n         beta = "true">\n         <NestedExtra>\n         </NestedExtra>\n      </FutureSetting>\n   </LaunchAction>`,
    ),
  ],
  [
    "unknown attribute on a known element preserved as extras",
    wrapScheme(
      `   <LaunchAction\n      buildConfiguration = "Debug"\n      futureXcodeFlag = "experimental"\n      anotherUnknown = "x">\n   </LaunchAction>`,
    ),
  ],
];

describe("parse → serialize: exact-output fidelity per element type", () => {
  it.each(ROUND_TRIP_CASES)("%s", (_label, xml) => {
    const doc = SchemeDocument.parse(xml);
    expect(doc.serialize()).toBe(xml);
  });
});

/**
 * Construction-from-scratch tests: build a tree using the OO API and assert
 * the serialized output exactly matches a hand-written expected XML. Together
 * with the parse → serialize tests above, these pin down both directions of
 * the wire format.
 */
describe("serialize from scratch: exact-output assertions", () => {
  it("empty SchemeDocument", () => {
    const doc = new SchemeDocument();
    expect(doc.serialize()).toBe('<?xml version="1.0" encoding="UTF-8"?>\n<Scheme>\n</Scheme>\n');
  });

  it("SchemeDocument with version + LastUpgradeVersion attrs", () => {
    const doc = new SchemeDocument();
    doc.lastUpgradeVersion = "1500";
    doc.version = "1.7";
    expect(doc.serialize()).toBe(
      `<?xml version="1.0" encoding="UTF-8"?>\n<Scheme\n   LastUpgradeVersion = "1500"\n   version = "1.7">\n</Scheme>\n`,
    );
  });

  it("LaunchAction with App Locale + one command-line arg + one env var", () => {
    const launch = new LaunchAction();
    launch.buildConfiguration = "Debug";
    launch.setAppLocale("ar", "SA");
    launch.addCommandLineArgument({ argument: "--verbose" });
    launch.addEnvironmentVariable({ key: "DEBUG", value: "1" });

    const doc = new SchemeDocument();
    doc.setLaunchAction(launch);

    expect(doc.serialize()).toBe(
      `<?xml version="1.0" encoding="UTF-8"?>\n<Scheme>\n   <LaunchAction\n      buildConfiguration = "Debug"\n      language = "ar"\n      region = "SA">\n      <CommandLineArguments>\n         <CommandLineArgument\n            argument = "--verbose"\n            isEnabled = "YES">\n         </CommandLineArgument>\n      </CommandLineArguments>\n      <EnvironmentVariables>\n         <EnvironmentVariable\n            key = "DEBUG"\n            value = "1"\n            isEnabled = "YES">\n         </EnvironmentVariable>\n      </EnvironmentVariables>\n   </LaunchAction>\n</Scheme>\n`,
    );
  });

  it("BuildAction with one BuildActionEntry referencing an app target", () => {
    const ref = new BuildableReference();
    ref.buildableIdentifier = "primary";
    ref.blueprintName = "MyApp";
    ref.buildableName = "MyApp.app";
    ref.referencedContainer = "container:MyApp.xcodeproj";

    const entry = new BuildActionEntry();
    entry.buildForTesting = true;
    entry.buildForRunning = true;
    entry.buildForProfiling = true;
    entry.buildForArchiving = true;
    entry.buildForAnalyzing = true;
    entry.setBuildableReference(ref);

    const build = new BuildAction();
    build.parallelizeBuildables = true;
    build.buildImplicitDependencies = true;
    build.addEntry(entry);

    const doc = new SchemeDocument();
    doc.setBuildAction(build);

    expect(doc.serialize()).toBe(
      `<?xml version="1.0" encoding="UTF-8"?>\n<Scheme>\n   <BuildAction\n      parallelizeBuildables = "YES"\n      buildImplicitDependencies = "YES">\n      <BuildActionEntries>\n         <BuildActionEntry\n            buildForTesting = "YES"\n            buildForRunning = "YES"\n            buildForProfiling = "YES"\n            buildForArchiving = "YES"\n            buildForAnalyzing = "YES">\n            <BuildableReference\n               BuildableIdentifier = "primary"\n               BlueprintName = "MyApp"\n               BuildableName = "MyApp.app"\n               ReferencedContainer = "container:MyApp.xcodeproj">\n            </BuildableReference>\n         </BuildActionEntry>\n      </BuildActionEntries>\n   </BuildAction>\n</Scheme>\n`,
    );
  });

  it("a comment added before LaunchAction appears in the right position", () => {
    const doc = new SchemeDocument();
    doc.setBuildAction(new BuildAction());
    doc.setLaunchAction(new LaunchAction());
    doc.addCommentBefore("LaunchAction", " RTL test build ");

    expect(doc.serialize()).toBe(
      `<?xml version="1.0" encoding="UTF-8"?>\n<Scheme>\n   <BuildAction>\n   </BuildAction>\n   <!-- RTL test build -->\n   <LaunchAction>\n   </LaunchAction>\n</Scheme>\n`,
    );
  });
});

/**
 * Recursive structural summary of a parsed scheme — attrs in order + slot
 * sequence, descending into typed children. Used by the differential
 * round-trip suite below to verify that a second parse produces semantically
 * the same tree as the first (catching e.g. a boolean coerced wrong on the
 * way back out).
 */
function snapNode(node: import("./xcscheme").SchemeNode): unknown {
  return {
    attrs: node.attributeEntries(),
    slots: node._slotsForSerialize().map((s) => {
      if (s.kind === "element") {
        const child = node._childAt(s.name, s.index);
        return { kind: "element", name: s.name, child: child ? snapNode(child) : null };
      }
      if (s.kind === "extra") return { kind: "extra", name: s.node.name };
      return { kind: s.kind, text: s.text };
    }),
  };
}

describe("differential round-trip: parse-after-serialize matches first parse", () => {
  it.each(FIXTURES)("%s — second parse produces identical structure", (name) => {
    const xml = loadFixture(name);
    const first = SchemeDocument.parse(xml);
    const re = SchemeDocument.parse(first.serialize());
    expect(snapNode(re)).toEqual(snapNode(first));
  });
});

describe("mutation round-trip — minimal diff", () => {
  it("changing only language and region produces exactly two diff lines", () => {
    const original = loadFixture("wikipedia-ios-rtl.xcscheme");
    const doc = SchemeDocument.parse(original);
    doc.launchAction()!.setAppLocale("ar", "SA");
    const mutated = doc.serialize();
    expect(mutated).toContain('language = "ar"');
    expect(mutated).toContain('region = "SA"');
    const origLines = original.split("\n");
    const mutLines = mutated.split("\n");
    expect(mutLines.length).toBe(origLines.length);
    let diffCount = 0;
    for (let i = 0; i < mutLines.length; i++) {
      if (mutLines[i] !== origLines[i]) diffCount++;
    }
    expect(diffCount).toBe(2);
  });
});

describe("construction from scratch", () => {
  it("builds a complete LaunchAction and serializes it", () => {
    const ref = new BuildableReference();
    ref.blueprintName = "MyApp";
    ref.buildableName = "MyApp.app";
    ref.referencedContainer = "container:MyApp.xcodeproj";

    const launch = new LaunchAction();
    launch.buildConfiguration = "Debug";
    launch.setAppLocale("ar", "SA");
    launch.addCommandLineArgument({ argument: "--verbose" });
    launch.addEnvironmentVariable({ key: "DEBUG", value: "1" });
    launch.enableAddressSanitizer = true;
    const runnable = new BuildableProductRunnable();
    runnable.setBuildableReference(ref);
    launch.setBuildableProductRunnable(runnable);

    const doc = new SchemeDocument();
    doc.version = "1.7";
    doc.setLaunchAction(launch);

    const xml = doc.serialize();
    expect(xml).toContain('language = "ar"');
    expect(xml).toContain('region = "SA"');
    expect(xml).toContain('enableAddressSanitizer = "YES"');
    expect(xml).toContain("MyApp.app");
    expect(xml).toContain("DEBUG");

    // Round-trip the constructed doc:
    const reparsed = SchemeDocument.parse(xml);
    expect(reparsed.launchAction()?.language).toBe("ar");
    expect(reparsed.launchAction()?.commandLineArguments()[0].argument).toBe("--verbose");
    expect(reparsed.launchAction()?.environmentVariables()[0].key).toBe("DEBUG");
  });
});

describe("escape handling", () => {
  it("round-trips XML entities in attribute values", () => {
    const doc = new SchemeDocument();
    const launch = new LaunchAction();
    launch.customWorkingDirectory = 'a & b < c > d "e" \n f';
    doc.setLaunchAction(launch);
    const xml = doc.serialize();
    expect(xml).toContain('customWorkingDirectory = "a &amp; b &lt; c &gt; d &quot;e&quot; &#10; f"');
    const reparsed = SchemeDocument.parse(xml);
    expect(reparsed.launchAction()?.customWorkingDirectory).toBe('a & b < c > d "e" \n f');
  });
});

describe("snapshots — representative fixtures", () => {
  const snapDir = path.resolve(__dirname, "__snapshots__");

  it("wikipedia-ios-rtl: RTL + args + SkippedTests + MacroExpansion", async () => {
    await expect(SchemeDocument.parse(loadFixture("wikipedia-ios-rtl.xcscheme")).serialize()).toMatchFileSnapshot(
      path.join(snapDir, "wikipedia-ios-rtl.xcscheme.snap"),
    );
  });

  it("alamofire-ios: TestPlans + AdditionalOptions + framework LaunchAction", async () => {
    await expect(SchemeDocument.parse(loadFixture("alamofire-ios.xcscheme")).serialize()).toMatchFileSnapshot(
      path.join(snapDir, "alamofire-ios.xcscheme.snap"),
    );
  });

  it("realm-swift: PreActions + shell-script ActionContent + entity escaping", async () => {
    await expect(SchemeDocument.parse(loadFixture("realm-swift.xcscheme")).serialize()).toMatchFileSnapshot(
      path.join(snapDir, "realm-swift.xcscheme.snap"),
    );
  });
});
