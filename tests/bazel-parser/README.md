# Bazel Parser

A functional TypeScript parser for extracting structured data from Bazel BUILD files, specifically designed for iOS projects using DoorDash's Bazel rules.

## Features

The parser extracts four key types of data from BUILD files:

- **xcschemes**: Build schemes from `xcodeproj` rules
- **xcode_configurations**: Xcode configuration references  
- **targets**: Library and binary targets (non-test)
- **targetsTest**: Test targets only

## Supported Bazel Rules

### 1. `xcodeproj` Rules
- Extracts `xcschemes` arrays with support for:
  - `doordash_scheme()` calls
  - `doordash_appclip_scheme()` calls  
  - `xcschemes.scheme()` calls with run configurations
- Parses `xcode_configurations` references
- Extracts `top_level_targets` as binary/test targets

### 2. `dd_ios_package` Rules
- Parses `target.library()` calls → library targets
- Parses `target.test()` calls → test targets
- Parses `target.binary()` calls → binary targets
- Extracts dependencies, paths, and resources

### 3. `cx_module` Rules
- Creates default library target using directory name
- Creates corresponding test target
- Handles feature flags and module configuration

## Usage

### Basic Parsing

```typescript
import { BazelParser } from './parser';

const buildFileContent = `
load("//bazel_support/rules:dd_ios_package.bzl", "dd_ios_package", "target")

dd_ios_package(
    name = "MyPackage",
    targets = [
        target.library(
            name = "MyLibrary",
            deps = [":OtherLib"],
            path = "Sources/MyLibrary",
        ),
        target.test(
            name = "MyLibraryTests", 
            deps = [":MyLibrary"],
            path = "Tests/MyLibraryTests",
        ),
    ],
)
`;

const result = BazelParser.parse(buildFileContent);

console.log('Libraries:', result.targets);
console.log('Tests:', result.targetsTest);
console.log('Schemes:', result.xcschemes);
console.log('Configurations:', result.xcode_configurations);
```

### Package-Level Parsing

```typescript
import { BazelParser } from './parser';

const packageInfo = BazelParser.parsePackage(
    buildFileContent,
    '/path/to/package'
);

console.log('Package:', packageInfo.name);
console.log('Path:', packageInfo.path);
console.log('Parse Results:', packageInfo.parseResult);
```

### Utility Functions

```typescript
import { BazelParserUtils } from './parser';

// Extract string arrays from Bazel syntax
const strings = BazelParserUtils.extractStringArray('["a", "b", "c"]');
// Result: ["a", "b", "c"]

// Extract key-value pairs
const dict = BazelParserUtils.extractDict('"key1": "value1", "key2": "value2"');
// Result: { key1: "value1", key2: "value2" }

// Find balanced parentheses content
const content = BazelParserUtils.findBalancedParens('func(arg1, arg2)', 4);
// Result: "arg1, arg2"
```

## Return Types

### BazelParseResult

```typescript
interface BazelParseResult {
  xcschemes: BazelScheme[];           // Build schemes
  xcode_configurations: BazelXcodeConfiguration[];  // Xcode configs
  targets: BazelTarget[];             // Library/binary targets
  targetsTest: BazelTarget[];         // Test targets
}
```

### BazelTarget

```typescript
interface BazelTarget {
  name: string;                       // Target name
  type: 'library' | 'test' | 'binary'; // Target type
  deps: string[];                     // Dependencies array
  path?: string;                      // Source path
  resources?: string[];               // Resources array
  buildLabel: string;                 // Full Bazel label (//package:target)
  testLabel?: string;                 // Test label (for test targets)
}
```

### BazelScheme

```typescript
interface BazelScheme {
  name: string;                       // Scheme name
  type: 'doordash_scheme' | 'doordash_appclip_scheme' | 'xcschemes_scheme' | 'custom';
  buildTargets: string[];             // Targets to build
  launchTarget?: string;              // Target to launch
  testTargets?: string[];             // Targets to test
  env?: Record<string, string>;       // Environment variables
  xcode_configuration?: string;       // Xcode configuration
}
```

## Testing

Run the test suite:

```bash
npm test tests/bazel-parser/
```

The tests include:
- Unit tests for each rule type
- Integration tests with sample BUILD files
- Edge cases and error handling
- Real-world complex scenarios

## Examples

See the `samples/` directory for example BUILD files:

- `xcodeproj.BUILD` - xcodeproj with schemes and configurations
- `dd_ios_package.BUILD` - Package with multiple library and test targets
- `cx_module.BUILD` - Simple cx_module definition

## Architecture

The parser uses a functional approach with:

- **Pattern Matching**: Regex-based parsing of Bazel syntax
- **Structured Extraction**: Separate parsers for each rule type
- **Robust Error Handling**: Graceful handling of malformed content
- **Type Safety**: Full TypeScript typing for all return values
- **Extensible Design**: Easy to add support for new rule types

## Limitations

- Assumes standard Bazel/Starlark syntax
- Does not evaluate Starlark code (static analysis only)
- Package path calculation is simplified (requires workspace root detection for production use)
- Some complex nested structures may need additional parsing logic

## Future Enhancements

- Support for more iOS-specific Bazel rules
- Dynamic Starlark evaluation
- Better workspace root detection
- Performance optimizations for large BUILD files
- Support for BUILD.bazel vs BUILD file naming
