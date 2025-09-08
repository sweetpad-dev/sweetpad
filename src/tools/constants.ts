export type Tool = {
  id: string;
  label: string;
  check: {
    command: string;
    args: string[];
  };
  install: {
    command: string;
    args: string[];
  };
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
      command: "brew",
      args: ["install", "--cask", "tuist"],
    },
    documentation: "https://docs.tuist.io/",
  },
  {
    id: "periphery",
    label: "Periphery",
    check: {
      command: "periphery",
      args: ["version"],
    },
    install: {
      command: "brew",
      args: ["install", "periphery"],
    },
    documentation: "https://github.com/peripheryapp/periphery",
  },
  {
    id: "bazel",
    label: "Bazel",
    check: {
      command: "bazel",
      args: ["--version"],
    },
    install: {
      command: "brew",
      args: ["install", "bazel"],
    },
    documentation: "https://bazel.build/",
  },
];
