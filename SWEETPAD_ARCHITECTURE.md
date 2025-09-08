# SweetPad Architecture Documentation

## Overview

SweetPad is a VSCode extension that enables iOS/Swift development within Visual Studio Code or Cursor. It provides a comprehensive suite of tools for building, testing, debugging, and managing iOS applications outside of Xcode.

## Project Structure

### Core Architecture

The extension follows a modular architecture with the main entry point in `src/extension.ts`. Each major feature is organized into its own module:

```
src/
├── extension.ts              # Main extension entry point
├── types.ts                  # Core type definitions
├── mcp_server.ts            # MCP (Model Context Protocol) server implementation
├── metrics.ts               # Metrics and analytics
├── common/                  # Shared utilities and infrastructure
├── build/                   # Build system functionality
│   └── bazel/               # Bazel integration and parsing
├── testing/                 # Test execution and management
├── debugger/                # Debug configuration and provider
├── simulators/              # iOS simulator management
├── devices/                 # Physical device management
├── destination/             # Build destination management
├── format/                  # Swift code formatting
├── tools/                   # Development tools management
├── system/                  # System commands and utilities
├── tuist/                   # Tuist integration
└── xcodegen/                # XcodeGen integration
```

## Core Components

### 1. Extension Entry Point (`extension.ts`)

The main activation function initializes all managers and registers commands:

- **BuildManager**: Handles build processes and configurations
- **DevicesManager**: Manages physical iOS devices  
- **SimulatorsManager**: Manages iOS simulators
- **DestinationsManager**: Coordinates build destinations
- **TestingManager**: Handles test execution
- **ToolsManager**: Manages development tools
- **MCP Server**: Provides AI integration capabilities

### 2. MCP Server Integration (`mcp_server.ts`)

The extension includes a Model Context Protocol (MCP) server that provides AI integration:

- **Purpose**: Enables AI assistants to execute VSCode commands
- **Protocol**: Uses Server-Sent Events (SSE) for real-time communication
- **Port**: Runs on port 61337 by default
- **Tools**: Exposes `executeCommand` tool for AI command execution

### 3. Common Infrastructure (`common/`)

Shared utilities and infrastructure:

- **commands.ts**: Command registration and execution framework
- **logger.ts**: Centralized logging system
- **exec.ts**: Command execution utilities
- **tasks.ts**: Task management and progress tracking
- **config.ts**: Configuration management
- **error-reporting.ts**: Error tracking and reporting

### 4. Build System (`build/`)

Comprehensive build management:

- **manager.ts**: Build orchestration and state management
- **commands.ts**: Build-related commands (build, run, clean, etc.)
- **provider.ts**: VSCode task provider for Xcode builds
- **tree.ts**: Workspace tree view provider
- **utils.ts**: Build utilities and helpers
- **bazel/**: Bazel build system integration

#### Bazel Integration (`build/bazel/`)

Comprehensive Bazel build support for iOS projects:

- **parser.ts**: Functional BUILD file parser for extracting structured data
  - Supports `xcodeproj`, `dd_ios_package`, `cx_module`, and `swift_library` rules
  - Extracts schemes, configurations, targets, and test targets
  - Handles DoorDash-specific Bazel rules (`doordash_scheme`, `doordash_appclip_scheme`)
- **types.ts**: Type definitions for Bazel entities
  - `BazelTarget`: Target definitions with build labels and dependencies
  - `BazelScheme`: Scheme definitions with launch configurations
  - `BazelXcodeConfiguration`: Xcode build configuration references
  - `BazelParseResult`: Complete parsing output structure
- **index.ts**: Main exports and public API
  - Provides `BazelParser` and utilities for BUILD file analysis
  - Backwards-compatible legacy format conversion

### 5. Testing Framework (`testing/`)

Test execution and management:

- **manager.ts**: Test orchestration and result processing
- **commands.ts**: Test-related commands
- **utils.ts**: Test utilities and helpers
- Supports both XCTest and Swift Testing frameworks

### 6. Debugger Integration (`debugger/`)

Debug configuration and provider:

- **provider.ts**: Debug configuration provider for CodeLLDB
- **commands.ts**: Debug-related commands
- **utils.ts**: Debug utilities

### 7. Device Management (`devices/` & `simulators/`)

Device and simulator management:

- **DevicesManager**: Physical device detection and management
- **SimulatorsManager**: iOS simulator lifecycle management
- **Commands**: Device/simulator control operations

### 8. Destination Management (`destination/`)

Build destination coordination:

- **manager.ts**: Destination selection and management
- **tree.ts**: Destination tree view
- **status-bar.ts**: Destination status bar integration
- **types.ts**: Destination type definitions

### 9. Code Formatting (`format/`)

Swift code formatting integration:

- **formatter.ts**: Swift formatting provider
- **commands.ts**: Format-related commands
- **status.ts**: Format status management

### 10. Tools Management (`tools/`)

Development tools management:

- **manager.ts**: Tool installation and management
- **commands.ts**: Tool-related commands
- **executeCommand.ts**: MCP command execution implementation
- **tree.ts**: Tools tree view

## Key Features

### Build & Run
- Xcode project/workspace detection
- Scheme and configuration selection
- Build destination management
- Build task provider integration
- Clean and rebuild operations

### Bazel Integration
- BUILD file parsing and target discovery
- Bazel scheme and configuration support
- Target selection and execution (build, test, run)
- DoorDash-specific Bazel rules support
- Multi-rule type parsing (`dd_ios_package`, `cx_module`, `xcodeproj`)
- Automatic build label generation and dependency resolution

### Testing
- XCTest and Swift Testing support
- Test target selection
- Test without building
- Test result processing
- Device/simulator testing

### Debugging
- CodeLLDB integration
- Debug configuration provider
- App path resolution
- Launch configuration management

### Device & Simulator Management
- iOS simulator lifecycle management
- Physical device detection
- Destination selection
- Cache management

### Code Formatting
- Swift-format integration
- Range formatting support
- Format status tracking
- Custom formatter configuration

### AI Integration (MCP)
- Command execution via AI
- Real-time communication
- Tool registration system
- Metrics and monitoring

### Periphery Scan Integration
- Unused code detection using Periphery tool
- Post-build automatic scanning option
- Context menu integration for schemes
- Configurable retention rules for public APIs

## Command Structure

Commands follow the pattern `sweetpad.{module}.{action}`:

```
sweetpad.build.run
sweetpad.build.clean
sweetpad.test.run
sweetpad.simulators.start
sweetpad.devices.refresh
sweetpad.format.run
sweetpad.tools.install
sweetpad.build.peripheryScan
sweetpad.build.buildAndPeripheryScan
sweetpad.build.createPeripheryConfig
sweetpad.bazel.build
sweetpad.bazel.test
sweetpad.bazel.run
sweetpad.bazel.selectTarget
sweetpad.bazel.buildSelected
sweetpad.bazel.testSelected
sweetpad.bazel.runSelected
```

## Configuration Management

The extension uses VSCode's configuration system with workspace-specific settings:

- Build configurations (scheme, destination, SDK)
- Tool preferences
- Format settings
- Debug configurations
- Recent destinations and usage statistics

## Error Handling

Comprehensive error handling system:

- **ErrorReporting**: Centralized error tracking
- **ExtensionError**: Custom error types
- **TaskError**: Task-specific error handling
- **Logger**: Structured logging with levels

## Task System

Advanced task management:

- **XcodeBuildTaskProvider**: VSCode task provider
- **Task execution**: Command execution with progress tracking
- **Task completion**: Event-driven completion notifications
- **Task cancellation**: Proper cleanup and cancellation

## AI Integration Architecture

The MCP server enables AI assistants to:

1. **Execute Commands**: Run any registered VSCode command
2. **Real-time Communication**: Use SSE for bidirectional communication
3. **Tool Registration**: Register custom tools for AI use
4. **Metrics Collection**: Track usage and performance
5. **Session Management**: Handle multiple AI sessions

## Periphery Scan Implementation

### Overview
The periphery scan feature provides unused code detection using the Periphery tool, integrated directly into the SweetPad build workflow.

### Core Components

#### 1. **Tool Management Integration**
- **Tool Registration**: Periphery added to `src/tools/constants.ts` for installation via Homebrew
- **Installation Command**: `brew install periphery`
- **Version Check**: `periphery version` command for validation

#### 2. **Scan Implementation (`src/build/commands.ts`)**
- **`runPeripheryScan()`**: Core function that executes periphery scan
- **`peripheryScanCommand()`**: Standalone periphery scan command
- **`buildAndPeripheryScanCommand()`**: Combined build and scan workflow
- **`createPeripheryConfigCommand()`**: Creates `.periphery.yml` configuration file template

#### 3. **Configuration Options**
```typescript
// Available configuration keys in workspace settings
"periphery.config": string              // Path to custom periphery config
"periphery.format": string              // Output format (default: "xcode")
"periphery.quiet": boolean              // Quiet mode toggle
"periphery.runAfterBuild": boolean      // Auto-run after builds
"periphery.retainPublic": boolean       // Retain public declarations
"periphery.retainObjcAccessible": boolean // Retain Objective-C accessible code
```

#### 4. **Command Structure**
```bash
periphery scan \
  --skip-build \
  --index-store-path /path/to/DerivedData/Index.noindex/DataStore \
  --retain-public \
  --retain-objc-accessible \
  --format xcode
```

### Implementation Details

#### **Index Store Path Resolution**
- **Primary**: Uses custom derived data path if configured
- **Fallback**: Searches `~/Library/Developer/Xcode/DerivedData/` for project-specific folders
- **Validation**: Checks path existence before execution
- **Error Handling**: Provides helpful messages and available folder listings

#### **Default Retention Rules**
- **`--retain-public`**: Preserves public APIs meant for external consumption
- **`--retain-objc-accessible`**: Preserves Objective-C accessible declarations
- **Configurable**: All rules can be disabled via workspace settings

#### **Integration Points**
1. **Build System**: Post-build automatic scanning via `periphery.runAfterBuild`
2. **Context Menu**: Right-click on schemes in tree view
3. **Command Palette**: Available via `sweetpad.build.peripheryScan` commands
4. **Tool Management**: Installation and version checking via tools system

#### **Error Handling**
- **Tool Availability**: Checks periphery installation before execution
- **Path Validation**: Ensures index store exists before scanning
- **Graceful Degradation**: Provides helpful error messages and suggestions
- **Non-zero Exit**: Handles periphery exit codes (findings vs errors)

### User Experience

#### **Context Menu Integration**
- Right-click on any scheme in the workspace tree
- Select "Periphery Scan" for quick analysis
- Select "Build & Periphery Scan" for full workflow

#### **Configuration Flexibility**
- Workspace-specific settings for different projects
- Custom periphery configuration file support
- Toggleable retention rules for different project types
- **Configuration Priority System**:
  1. `.periphery.yml` in project root (checked first)
  2. Custom path specified in `periphery.config` setting
  3. User-prompted path selection
  4. Default settings if no configuration is found
- **Template Generation**: Creates comprehensive `.periphery.yml` with project-specific defaults

#### **Output Integration**
- Xcode-compatible format for issue navigation
- Terminal output with progress indicators
- Automatic derived data cleanup suggestions

## Extension Lifecycle

1. **Activation**: Initialize managers and register commands
2. **Workspace Detection**: Detect Xcode projects/workspaces
3. **Service Registration**: Register providers and tree views
4. **MCP Server Start**: Initialize AI integration server
5. **Event Handling**: Process user interactions and commands
6. **Deactivation**: Clean up resources and close connections

## Dependencies

### Core Dependencies
- **VSCode Extension API**: Core extension functionality
- **Node.js**: Runtime environment
- **TypeScript**: Type-safe development

### MCP Integration
- **@modelcontextprotocol/sdk**: MCP server implementation
- **Express**: HTTP server for MCP endpoints
- **Zod**: Schema validation

### Build Tools
- **xcodebuild**: iOS build system
- **swift-format**: Code formatting
- **devicectl**: Device management
- **bazel**: Bazel build system

### Runtime Dependencies
- **@bacons/xcode**: Xcode project manipulation
- **@rgrove/parse-xml**: XML parsing for project files
- **@sentry/node**: Error reporting and monitoring
- **execa**: Process execution utilities
- **shell-quote**: Shell command escaping
- **vscode-languageclient**: Language server integration

## Bazel Implementation Architecture

### Overview

The Bazel integration provides comprehensive support for iOS projects using Bazel as the build system. It consists of a robust parsing system, target management, and seamless integration with the existing VSCode workflow.

### Bazel Parser System

#### Core Components

1. **BazelParser Class** (`src/build/bazel/parser.ts`)
   - **Static Parsing API**: Functional approach using static methods
   - **Multi-Rule Support**: Handles various Bazel rule types
   - **Balanced Parsing**: Uses sophisticated parentheses and bracket matching for nested structures
   - **Error Resilience**: Graceful handling of malformed BUILD files

2. **Supported Bazel Rules**
   - **`xcodeproj`**: Extracts schemes and configurations from Xcode project definitions
   - **`dd_ios_package`**: DoorDash-specific package definitions with target specifications
   - **`cx_module`**: Module definitions with automatic test target generation
   - **`swift_library`**: Standard Swift library targets
   - **`dd_ios_application`**: DoorDash application target definitions

3. **Parsing Strategy**
   - **Regex-Based Extraction**: Uses advanced regex patterns for rule identification
   - **Context-Aware Parsing**: Maintains file path context for accurate label generation
   - **Dependency Resolution**: Extracts and processes target dependencies
   - **Resource Mapping**: Identifies and catalogues resource dependencies

### Target Management

#### Target Types and Labels

1. **Build Labels**: Automatic generation of Bazel build labels (`//path/to/package:target`)
2. **Test Labels**: Special handling for test targets with appropriate labeling
3. **Dependency Mapping**: Cross-references between targets and their dependencies

#### Scheme Integration

- **DoorDash Schemes**: Special handling for `doordash_scheme()` and `doordash_appclip_scheme()`
- **Environment Variables**: Extraction and management of scheme-specific environment variables
- **Launch Configurations**: Mapping of schemes to their corresponding launch targets

### Workspace Integration

#### Tree View Integration

- **BazelTreeItem**: Specialized tree items for Bazel targets
- **WorkspaceTreeProvider**: Enhanced to display Bazel packages and targets
- **BazelTargetStatusBar**: Status bar integration for selected Bazel targets

#### Command Integration

- **Target Selection**: Interactive target selection with filtering capabilities
- **Build Commands**: Direct integration with `bazel build` commands
- **Test Execution**: Support for `bazel test` with proper target resolution
- **Run Commands**: Application launch via `bazel run`

### Execution Flow

1. **Discovery Phase**
   - Scan workspace for `BUILD.bazel` files
   - Parse each file using the comprehensive parser
   - Build internal target database

2. **Resolution Phase**
   - Resolve target dependencies
   - Generate appropriate build labels
   - Map schemes to targets

3. **Execution Phase**
   - Convert user selections to Bazel commands
   - Execute with appropriate flags and configurations
   - Handle output and error reporting

### Backwards Compatibility

The system maintains backwards compatibility through:
- **Legacy Format Conversion**: Automatic conversion between old and new target formats
- **API Preservation**: Existing interfaces remain unchanged
- **Gradual Migration**: Seamless transition from legacy parsing to new comprehensive system

This architecture provides a robust foundation for iOS development in VSCode while maintaining extensibility and AI integration capabilities. 