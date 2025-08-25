import * as vscode from 'vscode';
import { commonLogger } from './common/logger';
import { ExtensionContext } from './common/commands';
import { McpServer, ToolCallback } from "@modelcontextprotocol/sdk/server/mcp.js";
import { SSEServerTransport } from "@modelcontextprotocol/sdk/server/sse.js"; 
import { ZodRawShape, z } from 'zod';
import express, { type Express, type Request, type Response } from 'express';
import { McpServerOptions, McpServerInstance, McpToolDefinition } from './types'; 
import { setupMetrics } from './metrics';
import { RequestHandlerExtra } from '@modelcontextprotocol/sdk/shared/protocol.js';
import http from 'http';
// executeCommand import removed - now using individual command tools 
import { 
  takeScreenshotSchema,
  takeScreenshotImplementation,
  TakeScreenshotArgs
} from './tools/screenshotTool';
import { CallToolResult } from '@modelcontextprotocol/sdk/types.js';

const SSE_ENDPOINT = '/sse';
const MESSAGES_ENDPOINT = '/messages';
const METRICS_ENDPOINT = '/metrics';

// Helper function to create simple command tools
const createCommandTool = (
  commandId: string, 
  toolName: string, 
  description: string, 
  extensionContext: ExtensionContext
) => {
  const schema = z.object({}).describe(`${description}`);
  
  const implementation = async (args: any, frameworkExtra: RequestHandlerExtra<any, any>): Promise<CallToolResult> => {
    const timeoutSeconds = 600;
    
    let eventListener: vscode.Disposable | undefined;
    const waitForCompletionPromise = new Promise<"completed">((resolve) => {
      eventListener = extensionContext.simpleTaskCompletionEmitter.event(() => {
        resolve("completed");
      });
    });

    const timeoutPromise = new Promise<"timeout">((resolve) => {
      setTimeout(() => resolve("timeout"), timeoutSeconds * 1000);
    });

    try {
      vscode.commands.executeCommand(commandId).then(
        () => {},
        (initError) => commonLogger.error(`Error returned from executeCommand ${commandId}`, { initError })
      );
    } catch (execError: any) {
      commonLogger.error(`Error initiating command ${commandId}`, { execError });
      eventListener?.dispose();
      return { content: [{ type: "text", text: `Error initiating ${commandId}: ${execError.message}` }], isError: true };
    }

    const raceResult = await Promise.race([waitForCompletionPromise, timeoutPromise]);
    eventListener?.dispose();

    if (raceResult === "timeout") {
      return { content: [{ type: 'text', text: `TIMEOUT after ${timeoutSeconds}s waiting for command ${commandId} to signal completion.` }], isError: true };
    } else {
      return { content: [{ type: 'text', text: `Command ${commandId} completed successfully. Check output: ${extensionContext.UI_LOG_PATH()}` }], isError: false };
    }
  };

  return { toolName, schema: schema.shape, implementation };
};

export function createMcpServer(options: McpServerOptions, extensionContext: ExtensionContext): McpServerInstance {
  const app = express();
  const server = new McpServer({
    name: options.name,
    version: options.version,
  });

  const transports: { [sessionId: string]: SSEServerTransport } = {};
  const metricsRegistry = setupMetrics();

  // --- Setup Routes --- 
  app.get(SSE_ENDPOINT, async (_: Request, res: Response) => {
    commonLogger.log(`Received GET ${SSE_ENDPOINT}`);
    try {
      // Set up keep-alive to prevent connection timeouts
      const keepAliveIntervalMs = 20000;
      const keepAliveTimer = setInterval(() => {
        try {
          res.write(": keep-alive\n\n");
        } catch (error) {
          commonLogger.error('Error writing keep-alive message', { error });
          clearInterval(keepAliveTimer);
        }
      }, keepAliveIntervalMs);

      const transport = new SSEServerTransport(MESSAGES_ENDPOINT, res);
      transports[transport.sessionId] = transport;
      commonLogger.log(`SSE Connection: sessionId=${transport.sessionId}`);
      
      res.on('close', () => {
        clearInterval(keepAliveTimer);
        delete transports[transport.sessionId];
      });
      
      await server.connect(transport);
    } catch (err) {
      commonLogger.error('Error handling SSE connection', { err });
      if (!res.headersSent) res.status(500).send('SSE Connection Error');
    }
  });

  app.post(MESSAGES_ENDPOINT, express.json(), async (req: Request, res: Response) => {
    const sessionId = req.query.sessionId as string;
    
    const transport = transports[sessionId];
    if (transport) {
      try {
        await transport.handlePostMessage(req, res, req.body);
      } catch (err) {
        commonLogger.error(`Error handling message for ${sessionId}`, { err });
        if (!res.headersSent) res.status(500).send('Error handling message');
      }
    } else {
      commonLogger.warn(`No transport found for sessionId ${sessionId}`);
      res.status(400).send('No transport found for sessionId');
    }
  });

  app.get(METRICS_ENDPOINT, async (_: Request, res: Response) => {
    try {
      res.set('Content-Type', metricsRegistry.contentType);
      const metrics = await metricsRegistry.metrics();
      res.end(metrics);
    } catch (err) {
      commonLogger.error('Error serving metrics', { err });
      res.status(500).send('Error serving metrics');
    }
  });

  // === BUILD COMMANDS ===
  const buildLaunch = createCommandTool("sweetpad.build.launch", "launch_ios_app", "Build and launch the iOS app", extensionContext);
  server.tool(buildLaunch.toolName, "Build and launch the iOS app", buildLaunch.schema, buildLaunch.implementation);

  const buildRun = createCommandTool("sweetpad.build.run", "run_ios_app", "Build and run the iOS app", extensionContext);
  server.tool(buildRun.toolName, "Build and run the iOS app", buildRun.schema, buildRun.implementation);

  const buildBuild = createCommandTool("sweetpad.build.build", "build_ios_project", "Build the iOS project without running", extensionContext);
  server.tool(buildBuild.toolName, "Build the iOS project without running", buildBuild.schema, buildBuild.implementation);

  const buildClean = createCommandTool("sweetpad.build.clean", "clean_build_artifacts", "Clean iOS build artifacts", extensionContext);
  server.tool(buildClean.toolName, "Clean iOS build artifacts", buildClean.schema, buildClean.implementation);

  const buildTest = createCommandTool("sweetpad.build.test", "run_unit_tests", "Run unit tests", extensionContext);
  server.tool(buildTest.toolName, "Run unit tests", buildTest.schema, buildTest.implementation);

  const buildTestSwiftTesting = createCommandTool("sweetpad.build.testWithSwiftTesting", "run_swift_testing_tests", "Run tests with Swift Testing framework", extensionContext);
  server.tool(buildTestSwiftTesting.toolName, "Run tests with Swift Testing framework", buildTestSwiftTesting.schema, buildTestSwiftTesting.implementation);

  const buildResolveDependencies = createCommandTool("sweetpad.build.resolveDependencies", "resolve_swift_packages", "Resolve Swift Package Manager dependencies", extensionContext);
  server.tool(buildResolveDependencies.toolName, "Resolve Swift Package Manager dependencies", buildResolveDependencies.schema, buildResolveDependencies.implementation);

  const buildRemoveBundleDir = createCommandTool("sweetpad.build.removeBundleDir", "clean_app_bundle", "Remove app bundle directory", extensionContext);
  server.tool(buildRemoveBundleDir.toolName, "Remove app bundle directory", buildRemoveBundleDir.schema, buildRemoveBundleDir.implementation);

  const buildGenerateBuildServerConfig = createCommandTool("sweetpad.build.generateBuildServerConfig", "setup_build_server", "Generate Xcode build server configuration", extensionContext);
  server.tool(buildGenerateBuildServerConfig.toolName, "Generate Xcode build server configuration", buildGenerateBuildServerConfig.schema, buildGenerateBuildServerConfig.implementation);

  const buildOpenXcode = createCommandTool("sweetpad.build.openXcode", "open_project_in_xcode", "Open iOS project in Xcode", extensionContext);
  server.tool(buildOpenXcode.toolName, "Open iOS project in Xcode", buildOpenXcode.schema, buildOpenXcode.implementation);

  const buildSelectXcodeWorkspace = createCommandTool("sweetpad.build.selectXcodeWorkspace", "select_xcode_workspace", "Select Xcode workspace file", extensionContext);
  server.tool(buildSelectXcodeWorkspace.toolName, "Select Xcode workspace file", buildSelectXcodeWorkspace.schema, buildSelectXcodeWorkspace.implementation);

  const buildSetDefaultScheme = createCommandTool("sweetpad.build.setDefaultScheme", "set_build_scheme", "Set default build scheme", extensionContext);
  server.tool(buildSetDefaultScheme.toolName, "Set default build scheme", buildSetDefaultScheme.schema, buildSetDefaultScheme.implementation);

  const buildSelectConfiguration = createCommandTool("sweetpad.build.selectConfiguration", "select_build_configuration", "Select build configuration (Debug/Release)", extensionContext);
  server.tool(buildSelectConfiguration.toolName, "Select build configuration (Debug/Release)", buildSelectConfiguration.schema, buildSelectConfiguration.implementation);

  const buildDiagnoseSetup = createCommandTool("sweetpad.build.diagnoseSetup", "diagnose_build_issues", "Diagnose iOS build setup issues", extensionContext);
  server.tool(buildDiagnoseSetup.toolName, "Diagnose iOS build setup issues", buildDiagnoseSetup.schema, buildDiagnoseSetup.implementation);

  const buildPeripheryScan = createCommandTool("sweetpad.build.peripheryScan", "scan_unused_code", "Run Periphery scan to find unused Swift code", extensionContext);
  server.tool(buildPeripheryScan.toolName, "Run Periphery scan to find unused Swift code", buildPeripheryScan.schema, buildPeripheryScan.implementation);

  const buildBuildAndPeripheryScan = createCommandTool("sweetpad.build.buildAndPeripheryScan", "build_and_scan_unused_code", "Build project and run Periphery scan", extensionContext);
  server.tool(buildBuildAndPeripheryScan.toolName, "Build project and run Periphery scan", buildBuildAndPeripheryScan.schema, buildBuildAndPeripheryScan.implementation);

  const buildCreatePeripheryConfig = createCommandTool("sweetpad.build.createPeripheryConfig", "create_periphery_config", "Create Periphery configuration file", extensionContext);
  server.tool(buildCreatePeripheryConfig.toolName, "Create Periphery configuration file", buildCreatePeripheryConfig.schema, buildCreatePeripheryConfig.implementation);

  // === TESTING COMMANDS ===
  const testingBuildForTesting = createCommandTool("sweetpad.testing.buildForTesting", "build_for_testing", "Build iOS project for testing without running tests", extensionContext);
  server.tool(testingBuildForTesting.toolName, "Build iOS project for testing without running tests", testingBuildForTesting.schema, testingBuildForTesting.implementation);

  const testingTestWithoutBuilding = createCommandTool("sweetpad.testing.testWithoutBuilding", "run_tests_without_building", "Run tests without building project", extensionContext);
  server.tool(testingTestWithoutBuilding.toolName, "Run tests without building project", testingTestWithoutBuilding.schema, testingTestWithoutBuilding.implementation);

  const testingSelectTarget = createCommandTool("sweetpad.testing.selectTarget", "select_test_target", "Select iOS testing target", extensionContext);
  server.tool(testingSelectTarget.toolName, "Select iOS testing target", testingSelectTarget.schema, testingSelectTarget.implementation);

  const testingSetDefaultScheme = createCommandTool("sweetpad.testing.setDefaultScheme", "set_testing_scheme", "Set default scheme for testing", extensionContext);
  server.tool(testingSetDefaultScheme.toolName, "Set default scheme for testing", testingSetDefaultScheme.schema, testingSetDefaultScheme.implementation);

  const testingSelectConfiguration = createCommandTool("sweetpad.testing.selectConfiguration", "select_testing_configuration", "Select configuration for testing (Debug/Release)", extensionContext);
  server.tool(testingSelectConfiguration.toolName, "Select configuration for testing (Debug/Release)", testingSelectConfiguration.schema, testingSelectConfiguration.implementation);

  // === DEBUGGING COMMANDS ===
  const debuggerGetAppPath = createCommandTool("sweetpad.debugger.getAppPath", "get_debug_app_path", "Get iOS app path for debugging", extensionContext);
  server.tool(debuggerGetAppPath.toolName, "Get iOS app path for debugging", debuggerGetAppPath.schema, debuggerGetAppPath.implementation);

  const debuggerDebuggingLaunch = createCommandTool("sweetpad.debugger.debuggingLaunch", "launch_app_with_debugger", "Launch iOS app with debugger attached", extensionContext);
  server.tool(debuggerDebuggingLaunch.toolName, "Launch iOS app with debugger attached", debuggerDebuggingLaunch.schema, debuggerDebuggingLaunch.implementation);

  const debuggerDebuggingRun = createCommandTool("sweetpad.debugger.debuggingRun", "run_app_with_debugger", "Run iOS app with debugger attached", extensionContext);
  server.tool(debuggerDebuggingRun.toolName, "Run iOS app with debugger attached", debuggerDebuggingRun.schema, debuggerDebuggingRun.implementation);

  const debuggerDebuggingBuild = createCommandTool("sweetpad.debugger.debuggingBuild", "build_for_debugging", "Build iOS app for debugging", extensionContext);
  server.tool(debuggerDebuggingBuild.toolName, "Build iOS app for debugging", debuggerDebuggingBuild.schema, debuggerDebuggingBuild.implementation);

  // === PROJECT GENERATION COMMANDS ===
  const xcodegenGenerate = createCommandTool("sweetpad.xcodegen.generate", "generate_xcode_project_xcodegen", "Generate Xcode project using XcodeGen", extensionContext);
  server.tool(xcodegenGenerate.toolName, "Generate Xcode project using XcodeGen", xcodegenGenerate.schema, xcodegenGenerate.implementation);

  const tuistGenerate = createCommandTool("sweetpad.tuist.generate", "generate_xcode_project_tuist", "Generate Xcode project using Tuist", extensionContext);
  server.tool(tuistGenerate.toolName, "Generate Xcode project using Tuist", tuistGenerate.schema, tuistGenerate.implementation);

  const tuistInstall = createCommandTool("sweetpad.tuist.install", "install_tuist_dependencies", "Install Tuist dependencies", extensionContext);
  server.tool(tuistInstall.toolName, "Install Tuist dependencies", tuistInstall.schema, tuistInstall.implementation);

  const tuistClean = createCommandTool("sweetpad.tuist.clean", "clean_tuist_cache", "Clean Tuist cache and artifacts", extensionContext);
  server.tool(tuistClean.toolName, "Clean Tuist cache and artifacts", tuistClean.schema, tuistClean.implementation);

  const tuistEdit = createCommandTool("sweetpad.tuist.edit", "edit_tuist_project", "Edit Tuist project configuration", extensionContext);
  server.tool(tuistEdit.toolName, "Edit Tuist project configuration", tuistEdit.schema, tuistEdit.implementation);

  // === FORMAT COMMANDS ===
  const formatRun = createCommandTool("sweetpad.format.run", "format_swift_code", "Format Swift code using swift-format", extensionContext);
  server.tool(formatRun.toolName, "Format Swift code using swift-format", formatRun.schema, formatRun.implementation);

  const formatShowLogs = createCommandTool("sweetpad.format.showLogs", "show_formatter_logs", "Show Swift formatter logs", extensionContext);
  server.tool(formatShowLogs.toolName, "Show Swift formatter logs", formatShowLogs.schema, formatShowLogs.implementation);

  // === SIMULATOR COMMANDS ===
  const simulatorsRefresh = createCommandTool("sweetpad.simulators.refresh", "refresh_ios_simulators", "Refresh iOS simulator list", extensionContext);
  server.tool(simulatorsRefresh.toolName, "Refresh iOS simulator list", simulatorsRefresh.schema, simulatorsRefresh.implementation);

  const simulatorsOpenSimulator = createCommandTool("sweetpad.simulators.openSimulator", "open_ios_simulator", "Open iOS Simulator app", extensionContext);
  server.tool(simulatorsOpenSimulator.toolName, "Open iOS Simulator app", simulatorsOpenSimulator.schema, simulatorsOpenSimulator.implementation);

  const simulatorsRemoveCache = createCommandTool("sweetpad.simulators.removeCache", "clear_simulator_cache", "Clear iOS simulator cache", extensionContext);
  server.tool(simulatorsRemoveCache.toolName, "Clear iOS simulator cache", simulatorsRemoveCache.schema, simulatorsRemoveCache.implementation);

  const simulatorsStart = createCommandTool("sweetpad.simulators.start", "start_ios_simulator", "Start specific iOS Simulator", extensionContext);
  server.tool(simulatorsStart.toolName, "Start specific iOS Simulator", simulatorsStart.schema, simulatorsStart.implementation);

  const simulatorsStop = createCommandTool("sweetpad.simulators.stop", "stop_ios_simulator", "Stop iOS Simulator", extensionContext);
  server.tool(simulatorsStop.toolName, "Stop iOS Simulator", simulatorsStop.schema, simulatorsStop.implementation);

  // === DEVICE COMMANDS ===
  const devicesRefresh = createCommandTool("sweetpad.devices.refresh", "refresh_ios_devices", "Refresh connected iOS device list", extensionContext);
  server.tool(devicesRefresh.toolName, "Refresh connected iOS device list", devicesRefresh.schema, devicesRefresh.implementation);

  // === DESTINATION COMMANDS ===
  const destinationsSelect = createCommandTool("sweetpad.destinations.select", "select_build_destination", "Select build destination (device or simulator)", extensionContext);
  server.tool(destinationsSelect.toolName, "Select build destination (device or simulator)", destinationsSelect.schema, destinationsSelect.implementation);

  const destinationsRemoveRecent = createCommandTool("sweetpad.destinations.removeRecent", "remove_recent_destination", "Remove recent build destination", extensionContext);
  server.tool(destinationsRemoveRecent.toolName, "Remove recent build destination", destinationsRemoveRecent.schema, destinationsRemoveRecent.implementation);

  const destinationsSelectForTesting = createCommandTool("sweetpad.destinations.selectForTesting", "select_testing_destination", "Select destination for testing", extensionContext);
  server.tool(destinationsSelectForTesting.toolName, "Select destination for testing", destinationsSelectForTesting.schema, destinationsSelectForTesting.implementation);

  // === TOOL COMMANDS ===
  const toolsInstall = createCommandTool("sweetpad.tools.install", "install_development_tool", "Install iOS development tool", extensionContext);
  server.tool(toolsInstall.toolName, "Install iOS development tool", toolsInstall.schema, toolsInstall.implementation);

  const toolsRefresh = createCommandTool("sweetpad.tools.refresh", "refresh_development_tools", "Refresh development tools list", extensionContext);
  server.tool(toolsRefresh.toolName, "Refresh development tools list", toolsRefresh.schema, toolsRefresh.implementation);

  const toolsDocumentation = createCommandTool("sweetpad.tools.documentation", "open_tool_documentation", "Open development tool documentation", extensionContext);
  server.tool(toolsDocumentation.toolName, "Open development tool documentation", toolsDocumentation.schema, toolsDocumentation.implementation);

  // === SYSTEM COMMANDS ===
  const systemResetSweetpadCache = createCommandTool("sweetpad.system.resetSweetpadCache", "reset_sweetpad_cache", "Reset SweetPad extension cache", extensionContext);
  server.tool(systemResetSweetpadCache.toolName, "Reset SweetPad extension cache", systemResetSweetpadCache.schema, systemResetSweetpadCache.implementation);

  const systemCreateIssueGeneric = createCommandTool("sweetpad.system.createIssue.generic", "create_github_issue", "Create GitHub issue for SweetPad", extensionContext);
  server.tool(systemCreateIssueGeneric.toolName, "Create GitHub issue for SweetPad", systemCreateIssueGeneric.schema, systemCreateIssueGeneric.implementation);

  const systemCreateIssueNoSchemes = createCommandTool("sweetpad.system.createIssue.noSchemes", "create_no_schemes_issue", "Create GitHub issue for missing schemes", extensionContext);
  server.tool(systemCreateIssueNoSchemes.toolName, "Create GitHub issue for missing schemes", systemCreateIssueNoSchemes.schema, systemCreateIssueNoSchemes.implementation);

  const systemTestErrorReporting = createCommandTool("sweetpad.system.testErrorReporting", "test_error_reporting", "Test SweetPad error reporting system", extensionContext);
  server.tool(systemTestErrorReporting.toolName, "Test SweetPad error reporting system", systemTestErrorReporting.schema, systemTestErrorReporting.implementation);

  const systemOpenTerminalPanel = createCommandTool("sweetpad.system.openTerminalPanel", "open_terminal_panel", "Open VSCode terminal panel", extensionContext);
  server.tool(systemOpenTerminalPanel.toolName, "Open VSCode terminal panel", systemOpenTerminalPanel.schema, systemOpenTerminalPanel.implementation);

  server.tool(
      "take_simulator_screenshot",
      "Takes a screenshot of running iOS simulator and returns as image context.",
      takeScreenshotSchema.shape,
      async (args: TakeScreenshotArgs, frameworkExtra: RequestHandlerExtra<any, any>): Promise<CallToolResult> => {
        return takeScreenshotImplementation(args, { extensionContext: extensionContext });
      }
  );

  const registerTool = <T extends ZodRawShape>(tool: McpToolDefinition<T>): void => {
    server.tool(tool.name, tool.description, tool.schema, tool.implementation);
  };

  return {
    app,
    server,
    registerTool,
    start: async () => {
      const port = options.port || 8000;
      return new Promise<Express>((resolve, reject) => {
        const httpServer = http.createServer(app);
        httpServer.on('error', (err) => {
            commonLogger.error('HTTP server listen error', { err });
            reject(err);
        });
        httpServer.listen(port, () => {
          commonLogger.log(`MCP server listening on port ${port}`);
          resolve(app);
        });
      });
    },
  };
}