// Main exports for Bazel parser
export { BazelParser, BazelParserUtils } from "./parser";
export type {
  BazelTarget,
  BazelScheme,
  BazelXcodeConfiguration,
  BazelParseResult,
  BazelPackageInfo,
} from "./types";

// Re-export for convenience
export * from "./parser";
export * from "./types";
