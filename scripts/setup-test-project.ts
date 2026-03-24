import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { execa } from "execa";

// ── Templates ──────────────────────────────────────────────────────────

interface ProjectTemplate {
  name: string;
  description: string;
  files: { path: string; content: string }[];
  /** Run after writing files. Runs from the project root directory. */
  postSetup?: { command: string; args: string[] }[];
}

const TEMPLATES: Record<string, ProjectTemplate> = {
  "ios-app": {
    name: "ios-app",
    description: "Single iOS app with one scheme (requires xcodegen)",
    files: [
      {
        path: "project.yml",
        content: `name: TestApp
options:
  bundleIdPrefix: com.sweetpad.test
  deploymentTarget:
    iOS: "16.0"
targets:
  TestApp:
    type: application
    platform: iOS
    sources: [Sources]
    settings:
      PRODUCT_BUNDLE_IDENTIFIER: com.sweetpad.test.app
`,
      },
      {
        path: "Sources/App.swift",
        content: `import SwiftUI

@main
struct TestApp: App {
    var body: some Scene {
        WindowGroup {
            Text("Hello")
        }
    }
}
`,
      },
    ],
    postSetup: [{ command: "xcodegen", args: ["generate"] }],
  },

  "multi-project": {
    name: "multi-project",
    description: "Workspace with two projects: app + framework (requires xcodegen)",
    files: [
      {
        path: "App/project.yml",
        content: `name: MainApp
options:
  bundleIdPrefix: com.sweetpad.test
  deploymentTarget:
    iOS: "16.0"
targets:
  MainApp:
    type: application
    platform: iOS
    sources: [Sources]
    dependencies:
      - framework: ../SharedKit/build/SharedKit.framework
`,
      },
      {
        path: "App/Sources/App.swift",
        content: `import SwiftUI

@main
struct MainApp: App {
    var body: some Scene {
        WindowGroup { Text("Main") }
    }
}
`,
      },
      {
        path: "SharedKit/project.yml",
        content: `name: SharedKit
options:
  bundleIdPrefix: com.sweetpad.test
  deploymentTarget:
    iOS: "16.0"
targets:
  SharedKit:
    type: framework
    platform: iOS
    sources: [Sources]
`,
      },
      {
        path: "SharedKit/Sources/Shared.swift",
        content: `public struct Shared {
    public static let version = "1.0"
}
`,
      },
      {
        path: "TestWorkspace.xcworkspace/contents.xcworkspacedata",
        content: `<?xml version="1.0" encoding="UTF-8"?>
<Workspace
   version = "1.0">
   <FileRef
      location = "group:App/MainApp.xcodeproj">
   </FileRef>
   <FileRef
      location = "group:SharedKit/SharedKit.xcodeproj">
   </FileRef>
</Workspace>
`,
      },
    ],
    postSetup: [
      { command: "xcodegen", args: ["generate", "--spec", "App/project.yml", "--project", "App"] },
      { command: "xcodegen", args: ["generate", "--spec", "SharedKit/project.yml", "--project", "SharedKit"] },
    ],
  },

  "spm-package": {
    name: "spm-package",
    description: "Swift Package Manager package (no xcodegen needed)",
    files: [
      {
        path: "Package.swift",
        content: `// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "TestPackage",
    products: [
        .library(name: "TestLib", targets: ["TestLib"]),
    ],
    targets: [
        .target(name: "TestLib"),
        .testTarget(name: "TestLibTests", dependencies: ["TestLib"]),
    ]
)
`,
      },
      {
        path: "Sources/TestLib/Lib.swift",
        content: `public struct TestLib {
    public static let version = "1.0"
    public init() {}
}
`,
      },
      {
        path: "Tests/TestLibTests/TestLibTests.swift",
        content: `import XCTest
@testable import TestLib

final class TestLibTests: XCTestCase {
    func testVersion() {
        XCTAssertEqual(TestLib.version, "1.0")
    }
}
`,
      },
    ],
  },

  "multi-platform": {
    name: "multi-platform",
    description: "Project with iOS, watchOS, tvOS targets (requires xcodegen)",
    files: [
      {
        path: "project.yml",
        content: `name: MultiPlatform
options:
  bundleIdPrefix: com.sweetpad.test
  deploymentTarget:
    iOS: "16.0"
    watchOS: "9.0"
    tvOS: "16.0"
targets:
  iOSApp:
    type: application
    platform: iOS
    sources: [Sources/iOS]
  WatchApp:
    type: application
    platform: watchOS
    sources: [Sources/watchOS]
  TVApp:
    type: application
    platform: tvOS
    sources: [Sources/tvOS]
`,
      },
      {
        path: "Sources/iOS/App.swift",
        content: `import SwiftUI
@main struct IOSApp: App {
    var body: some Scene { WindowGroup { Text("iOS") } }
}
`,
      },
      {
        path: "Sources/watchOS/App.swift",
        content: `import SwiftUI
@main struct WatchApp: App {
    var body: some Scene { WindowGroup { Text("watch") } }
}
`,
      },
      {
        path: "Sources/tvOS/App.swift",
        content: `import SwiftUI
@main struct TVApp: App {
    var body: some Scene { WindowGroup { Text("TV") } }
}
`,
      },
    ],
    postSetup: [{ command: "xcodegen", args: ["generate"] }],
  },
};

// ── Scaffold ───────────────────────────────────────────────────────────

async function scaffoldProject(template: ProjectTemplate, outputDir: string): Promise<void> {
  console.log(`\nScaffolding "${template.name}" → ${outputDir}\n`);

  // Write all files
  for (const file of template.files) {
    const fullPath = join(outputDir, file.path);
    mkdirSync(join(fullPath, ".."), { recursive: true });
    writeFileSync(fullPath, file.content, "utf-8");
    console.log(`  ✓ ${file.path}`);
  }

  // Run post-setup commands
  if (template.postSetup) {
    for (const step of template.postSetup) {
      console.log(`\n  Running: ${step.command} ${step.args.join(" ")}`);
      try {
        await execa(step.command, step.args, { cwd: outputDir, stdio: "inherit" });
      } catch (e: any) {
        console.error(`  ✗ Command failed: ${e.message}`);
        if (step.command === "xcodegen") {
          console.error("\n  Install xcodegen: brew install xcodegen");
        }
        process.exit(1);
      }
    }
  }

  console.log(`\nDone. Project created at: ${outputDir}`);
}

// ── Main ───────────────────────────────────────────────────────────────

function usage(): never {
  console.log(`Usage: tsx scripts/setup-test-project.ts <template> [--output <dir>]

Scaffolds a minimal Xcode project for reproducing bugs.

Templates:`);
  for (const [key, tmpl] of Object.entries(TEMPLATES)) {
    console.log(`  ${key.padEnd(20)} ${tmpl.description}`);
  }
  console.log(`
Options:
  --output <dir>    Output directory (default: tests/fixtures/projects/<template>)
  --list            List available templates

Examples:
  tsx scripts/setup-test-project.ts ios-app
  tsx scripts/setup-test-project.ts multi-project --output /tmp/bug-123
  tsx scripts/setup-test-project.ts spm-package`);
  process.exit(1);
}

async function main() {
  const args = process.argv.slice(2);

  if (args.length === 0 || args.includes("--help")) {
    usage();
  }

  if (args.includes("--list")) {
    console.log("Available templates:\n");
    for (const [key, tmpl] of Object.entries(TEMPLATES)) {
      console.log(`  ${key.padEnd(20)} ${tmpl.description}`);
    }
    process.exit(0);
  }

  const templateName = args[0];
  const template = TEMPLATES[templateName];
  if (!template) {
    console.error(`Unknown template: "${templateName}". Use --list to see available templates.`);
    process.exit(1);
  }

  // Check xcodegen availability if needed
  if (template.postSetup?.some((s) => s.command === "xcodegen")) {
    try {
      await execa("which", ["xcodegen"]);
    } catch {
      console.error("Error: xcodegen is required but not installed.");
      console.error("Install it with: brew install xcodegen");
      process.exit(1);
    }
  }

  const outputIndex = args.indexOf("--output");
  const outputDir =
    outputIndex !== -1 && args[outputIndex + 1]
      ? resolve(args[outputIndex + 1])
      : join(process.cwd(), "tests", "fixtures", "projects", templateName);

  if (existsSync(outputDir)) {
    console.log(`Directory already exists: ${outputDir}`);
    console.log("Remove it first or use --output to specify a different path.");
    process.exit(1);
  }

  mkdirSync(outputDir, { recursive: true });
  await scaffoldProject(template, outputDir);
}

void main().catch((e) => {
  console.error("Fatal error:", e);
  process.exit(1);
});
