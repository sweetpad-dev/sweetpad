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

This architecture provides a robust foundation for iOS development in VSCode while maintaining extensibility and AI integration capabilities. 