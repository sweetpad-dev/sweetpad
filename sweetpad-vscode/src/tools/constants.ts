export type ToolInstall = { type: "shell"; command: string; args: string[] } | { type: "openUrl"; url: string };

export type Tool = {
  id: string;
  label: string;
  check: {
    command: string;
    args: string[];
  };
  install: ToolInstall;
  documentation: string;
};

export const TOOLS: Tool[] = [
  {
    id: "brew",
    label: "Homebrew",
    check: {
      command: "brew",
      args: ["--version"],
    },
    install: {
      type: "shell",
      command: "/bin/bash",
      args: ["-c", "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"],
    },
    documentation: "https://brew.sh/",
  },
  {
    id: "swift-format",
    label: "swift-format",
    check: {
      command: "swift-format",
      args: ["--version"],
    },
    install: {
      type: "shell",
      command: "brew",
      args: ["install", "swift-format"],
    },
    documentation: "https://github.com/apple/swift-format",
  },
  {
    id: "xcodegen",
    label: "XcodeGen",
    check: {
      command: "xcodegen",
      args: ["--version"],
    },
    install: {
      type: "shell",
      command: "brew",
      args: ["install", "xcodegen"],
    },
    documentation: "https://github.com/yonaskolb/XcodeGen",
  },
  {
    id: "swiftlint",
    label: "SwiftLint",
    check: {
      command: "swiftlint",
      args: ["--version"],
    },
    install: {
      type: "shell",
      command: "brew",
      args: ["install", "swiftlint"],
    },
    documentation: "https://github.com/realm/SwiftLint",
  },
  {
    id: "xcbeautify",
    label: "xcbeautify",
    check: {
      command: "xcbeautify",
      args: ["--version"],
    },
    install: {
      type: "shell",
      command: "brew",
      args: ["install", "xcbeautify"],
    },
    documentation: "https://github.com/cpisciotta/xcbeautify",
  },
  {
    id: "xcode-build-server",
    label: "xcode-build-server",
    check: {
      command: "xcode-build-server",
      args: ["--help"],
    },
    install: {
      type: "shell",
      command: "brew",
      args: ["install", "xcode-build-server"],
    },
    documentation: "https://github.com/SolaWing/xcode-build-server",
  },
  {
    id: "ios-deploy",
    label: "ios-deploy",
    check: {
      command: "ios-deploy",
      args: ["--version"],
    },
    install: {
      type: "shell",
      command: "brew",
      args: ["install", "ios-deploy"],
    },
    documentation: "https://github.com/ios-control/ios-deploy",
  },
  {
    id: "tuist",
    label: "tuist",
    check: {
      command: "tuist",
      args: ["version"],
    },
    install: {
      type: "shell",
      command: "brew",
      args: ["install", "--cask", "tuist"],
    },
    documentation: "https://docs.tuist.io/",
  },
  {
    id: "injectionnext",
    label: "InjectionNext",
    check: {
      command: "test",
      args: ["-d", "/Applications/InjectionNext.app"],
    },
    // InjectionNext is not on Homebrew; the install action opens the GitHub releases
    // page so the user can grab the ZIP and drop InjectionNext.app into /Applications.
    install: {
      type: "openUrl",
      url: "https://github.com/johnno1962/InjectionNext/releases/latest",
    },
    documentation: "https://github.com/johnno1962/InjectionNext",
  },
];

/** Look up a tool by its `id` (throws on an unknown id — caller passes a literal). */
export function getToolById(id: string): Tool {
  const tool = TOOLS.find((t) => t.id === id);
  if (!tool) {
    throw new Error(`Unknown tool id: ${id}`);
  }
  return tool;
}
