# Swift Testing Support

SweetPad now supports running tests with the Swift Testing framework in addition to XCTest, including inline test running capabilities.

## Overview

Swift Testing is Apple's new testing framework introduced with Swift 5.10 and Xcode 16. It provides a more modern and expressive way to write tests using the `@Test` macro and other features.

## Features

- **Automatic Test Discovery**: SweetPad automatically detects Swift Testing tests in your code
- **Inline Test Running**: Click on test names in the editor to run individual tests (just like XCTest)
- **Test Explorer Integration**: Swift Testing tests appear in VS Code's Test Explorer
- **Mixed Framework Support**: Use both XCTest and Swift Testing in the same project

## Configuration

You can configure which testing framework to use in your VSCode settings:

```json
{
  "sweetpad.testing.framework": "auto" // Options: "xctest", "swift-testing", "auto"
}
```

### Options:
- **`auto`** (default): Automatically detects which framework to use based on your test files
- **`xctest`**: Always use XCTest framework
- **`swift-testing`**: Always use Swift Testing framework (requires Xcode 16+)

## Running Tests

### Method 1: Inline Test Running

SweetPad automatically discovers Swift Testing tests in your code and provides CodeLens actions:
- Click on a test name to run it
- Right-click for more options
- Tests are marked with "(Swift Testing)" in the test explorer

### Method 2: Using Commands

1. **Run Tests (Auto-detect)**: Use the command `SweetPad: Test` to run tests with the configured framework
2. **Run Swift Testing Explicitly**: Use the command `SweetPad: Test with Swift Testing` to force using Swift Testing

### Method 3: From the Build Tree

Right-click on a scheme in the SweetPad build tree and select:
- **Test** - Uses the configured framework
- **Test with Swift Testing** - Forces Swift Testing

### Method 4: Test Explorer

Swift Testing tests appear in VS Code's Test Explorer panel where you can:
- Run individual tests
- Run test suites
- Debug tests (if supported)

## Swift Package Manager (SPM) Projects

For SPM projects, when Swift Testing is selected:
- Uses `swift test` command instead of `xcodebuild test`
- Automatically handles the correct configuration and target selection
- Supports test filtering with `--filter` flag

## Xcode Projects

For Xcode projects (.xcworkspace/.xcodeproj):
- Uses `xcodebuild` with `-testLanguage swift` flag
- Supports test selection with `-testIdentifier`
- Integrates with existing build configurations

## Example Test File

```swift
import Testing

// Struct-based test suite
struct MyTests {
    @Test func example() async throws {
        // Write your test here using APIs like `#expect(...)`
        #expect(1 + 1 == 2)
    }
    
    @Test("Custom test description")
    func customTest() {
        #expect(true)
    }
}

// Standalone test function
@Test func standaloneTest() {
    #expect(2 + 2 == 4)
}

// Class-based test suite (also supported)
class MyClassTests {
    @Test func classBasedTest() {
        #expect("Hello" == "Hello")
    }
}
```

## Test Discovery

SweetPad automatically discovers:
- Functions decorated with `@Test`
- Test suites in `struct`, `class`, or `actor` types
- Standalone test functions
- Parameterized tests with `@Test(arguments:)`

## Requirements

- Xcode 16 or later for Swift Testing support
- Swift 5.10 or later
- macOS 14.0 or later recommended

## Troubleshooting

If Swift Testing tests are not running:
1. Ensure you have Xcode 16+ installed
2. Check that your project targets Swift 5.10+
3. Verify the `sweetpad.testing.framework` setting
4. For SPM projects, ensure your Package.swift includes Testing as a dependency
5. Check that test files are properly saved before running

### Known Limitations

- Test output parsing may vary between Swift Testing versions
- Some advanced Swift Testing features may not be fully supported
- Debugging Swift Testing tests requires additional setup 