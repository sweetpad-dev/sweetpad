// Types for Bazel parser output
export interface BazelTarget {
  name: string;
  type: "library" | "test" | "binary";
  deps: string[];
  path?: string;
  resources?: string[];
  buildLabel: string;
  testLabel?: string;
}

export interface BazelScheme {
  name: string;
  type: "doordash_scheme" | "doordash_appclip_scheme" | "xcschemes_scheme" | "custom";
  buildTargets: string[];
  launchTarget?: string;
  testTargets?: string[];
  env?: Record<string, string>;
  xcode_configuration?: string;
}

export interface BazelXcodeConfiguration {
  name: string;
  buildSettings?: Record<string, any>;
}

export interface BazelParseResult {
  xcschemes: BazelScheme[];
  xcode_configurations: BazelXcodeConfiguration[];
  targets: BazelTarget[];
  targetsTest: BazelTarget[];
}

export interface BazelPackageInfo {
  name: string;
  path: string;
  parseResult: BazelParseResult;
}
