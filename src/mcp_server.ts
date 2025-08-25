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
  const buildLaunch = createCommandTool("sweetpad.build.launch", "build_launch", "Build and launch the app", extensionContext);
  server.tool(buildLaunch.toolName, "Build and launch the app", buildLaunch.schema, buildLaunch.implementation);

  const buildRun = createCommandTool("sweetpad.build.run", "build_run", "Build and run the app", extensionContext);
  server.tool(buildRun.toolName, "Build and run the app", buildRun.schema, buildRun.implementation);

  const buildBuild = createCommandTool("sweetpad.build.build", "build_build", "Build the app without running", extensionContext);
  server.tool(buildBuild.toolName, "Build the app without running", buildBuild.schema, buildBuild.implementation);

  const buildClean = createCommandTool("sweetpad.build.clean", "build_clean", "Clean build artifacts", extensionContext);
  server.tool(buildClean.toolName, "Clean build artifacts", buildClean.schema, buildClean.implementation);

  const buildTest = createCommandTool("sweetpad.build.test", "build_test", "Run tests", extensionContext);
  server.tool(buildTest.toolName, "Run tests", buildTest.schema, buildTest.implementation);

  const buildTestSwiftTesting = createCommandTool("sweetpad.build.testWithSwiftTesting", "build_test_swift_testing", "Run tests with Swift Testing framework", extensionContext);
  server.tool(buildTestSwiftTesting.toolName, "Run tests with Swift Testing framework", buildTestSwiftTesting.schema, buildTestSwiftTesting.implementation);

  const buildResolveDependencies = createCommandTool("sweetpad.build.resolveDependencies", "build_resolve_dependencies", "Resolve package dependencies", extensionContext);
  server.tool(buildResolveDependencies.toolName, "Resolve package dependencies", buildResolveDependencies.schema, buildResolveDependencies.implementation);

  const buildRemoveBundleDir = createCommandTool("sweetpad.build.removeBundleDir", "build_remove_bundle_dir", "Remove build bundle directory", extensionContext);
  server.tool(buildRemoveBundleDir.toolName, "Remove build bundle directory", buildRemoveBundleDir.schema, buildRemoveBundleDir.implementation);

  const buildGenerateBuildServerConfig = createCommandTool("sweetpad.build.generateBuildServerConfig", "build_generate_build_server_config", "Generate build server configuration", extensionContext);
  server.tool(buildGenerateBuildServerConfig.toolName, "Generate build server configuration", buildGenerateBuildServerConfig.schema, buildGenerateBuildServerConfig.implementation);

  const buildOpenXcode = createCommandTool("sweetpad.build.openXcode", "build_open_xcode", "Open project in Xcode", extensionContext);
  server.tool(buildOpenXcode.toolName, "Open project in Xcode", buildOpenXcode.schema, buildOpenXcode.implementation);

  const buildSelectXcodeWorkspace = createCommandTool("sweetpad.build.selectXcodeWorkspace", "build_select_xcode_workspace", "Select Xcode workspace", extensionContext);
  server.tool(buildSelectXcodeWorkspace.toolName, "Select Xcode workspace", buildSelectXcodeWorkspace.schema, buildSelectXcodeWorkspace.implementation);

  const buildSetDefaultScheme = createCommandTool("sweetpad.build.setDefaultScheme", "build_set_default_scheme", "Set default build scheme", extensionContext);
  server.tool(buildSetDefaultScheme.toolName, "Set default build scheme", buildSetDefaultScheme.schema, buildSetDefaultScheme.implementation);

  const buildSelectConfiguration = createCommandTool("sweetpad.build.selectConfiguration", "build_select_configuration", "Select build configuration", extensionContext);
  server.tool(buildSelectConfiguration.toolName, "Select build configuration", buildSelectConfiguration.schema, buildSelectConfiguration.implementation);

  const buildDiagnoseSetup = createCommandTool("sweetpad.build.diagnoseSetup", "build_diagnose_setup", "Diagnose build setup issues", extensionContext);
  server.tool(buildDiagnoseSetup.toolName, "Diagnose build setup issues", buildDiagnoseSetup.schema, buildDiagnoseSetup.implementation);

  const buildPeripheryScan = createCommandTool("sweetpad.build.peripheryScan", "build_periphery_scan", "Run Periphery scan for unused code", extensionContext);
  server.tool(buildPeripheryScan.toolName, "Run Periphery scan for unused code", buildPeripheryScan.schema, buildPeripheryScan.implementation);

  const buildBuildAndPeripheryScan = createCommandTool("sweetpad.build.buildAndPeripheryScan", "build_build_and_periphery_scan", "Build and run Periphery scan", extensionContext);
  server.tool(buildBuildAndPeripheryScan.toolName, "Build and run Periphery scan", buildBuildAndPeripheryScan.schema, buildBuildAndPeripheryScan.implementation);

  const buildCreatePeripheryConfig = createCommandTool("sweetpad.build.createPeripheryConfig", "build_create_periphery_config", "Create Periphery configuration file", extensionContext);
  server.tool(buildCreatePeripheryConfig.toolName, "Create Periphery configuration file", buildCreatePeripheryConfig.schema, buildCreatePeripheryConfig.implementation);

  // === TESTING COMMANDS ===
  const testingBuildForTesting = createCommandTool("sweetpad.testing.buildForTesting", "testing_build_for_testing", "Build for testing without running tests", extensionContext);
  server.tool(testingBuildForTesting.toolName, "Build for testing without running tests", testingBuildForTesting.schema, testingBuildForTesting.implementation);

  const testingTestWithoutBuilding = createCommandTool("sweetpad.testing.testWithoutBuilding", "testing_test_without_building", "Run tests without building", extensionContext);
  server.tool(testingTestWithoutBuilding.toolName, "Run tests without building", testingTestWithoutBuilding.schema, testingTestWithoutBuilding.implementation);

  const testingSelectTarget = createCommandTool("sweetpad.testing.selectTarget", "testing_select_target", "Select testing target", extensionContext);
  server.tool(testingSelectTarget.toolName, "Select testing target", testingSelectTarget.schema, testingSelectTarget.implementation);

  const testingSetDefaultScheme = createCommandTool("sweetpad.testing.setDefaultScheme", "testing_set_default_scheme", "Set default testing scheme", extensionContext);
  server.tool(testingSetDefaultScheme.toolName, "Set default testing scheme", testingSetDefaultScheme.schema, testingSetDefaultScheme.implementation);

  const testingSelectConfiguration = createCommandTool("sweetpad.testing.selectConfiguration", "testing_select_configuration", "Select testing configuration", extensionContext);
  server.tool(testingSelectConfiguration.toolName, "Select testing configuration", testingSelectConfiguration.schema, testingSelectConfiguration.implementation);

  // === DEBUGGING COMMANDS ===
  const debuggerGetAppPath = createCommandTool("sweetpad.debugger.getAppPath", "debugger_get_app_path", "Get app path for debugging", extensionContext);
  server.tool(debuggerGetAppPath.toolName, "Get app path for debugging", debuggerGetAppPath.schema, debuggerGetAppPath.implementation);

  const debuggerDebuggingLaunch = createCommandTool("sweetpad.debugger.debuggingLaunch", "debugger_debugging_launch", "Launch app with debugger", extensionContext);
  server.tool(debuggerDebuggingLaunch.toolName, "Launch app with debugger", debuggerDebuggingLaunch.schema, debuggerDebuggingLaunch.implementation);

  const debuggerDebuggingRun = createCommandTool("sweetpad.debugger.debuggingRun", "debugger_debugging_run", "Run app with debugger", extensionContext);
  server.tool(debuggerDebuggingRun.toolName, "Run app with debugger", debuggerDebuggingRun.schema, debuggerDebuggingRun.implementation);

  const debuggerDebuggingBuild = createCommandTool("sweetpad.debugger.debuggingBuild", "debugger_debugging_build", "Build app for debugging", extensionContext);
  server.tool(debuggerDebuggingBuild.toolName, "Build app for debugging", debuggerDebuggingBuild.schema, debuggerDebuggingBuild.implementation);

  // === PROJECT GENERATION COMMANDS ===
  const xcodegenGenerate = createCommandTool("sweetpad.xcodegen.generate", "xcodegen_generate", "Generate Xcode project using XcodeGen", extensionContext);
  server.tool(xcodegenGenerate.toolName, "Generate Xcode project using XcodeGen", xcodegenGenerate.schema, xcodegenGenerate.implementation);

  const tuistGenerate = createCommandTool("sweetpad.tuist.generate", "tuist_generate", "Generate Xcode project using Tuist", extensionContext);
  server.tool(tuistGenerate.toolName, "Generate Xcode project using Tuist", tuistGenerate.schema, tuistGenerate.implementation);

  const tuistInstall = createCommandTool("sweetpad.tuist.install", "tuist_install", "Install Tuist dependencies", extensionContext);
  server.tool(tuistInstall.toolName, "Install Tuist dependencies", tuistInstall.schema, tuistInstall.implementation);

  const tuistClean = createCommandTool("sweetpad.tuist.clean", "tuist_clean", "Clean Tuist cache", extensionContext);
  server.tool(tuistClean.toolName, "Clean Tuist cache", tuistClean.schema, tuistClean.implementation);

  const tuistEdit = createCommandTool("sweetpad.tuist.edit", "tuist_edit", "Edit Tuist project", extensionContext);
  server.tool(tuistEdit.toolName, "Edit Tuist project", tuistEdit.schema, tuistEdit.implementation);

  // === FORMAT COMMANDS ===
  const formatRun = createCommandTool("sweetpad.format.run", "format_run", "Format Swift code", extensionContext);
  server.tool(formatRun.toolName, "Format Swift code", formatRun.schema, formatRun.implementation);

  const formatShowLogs = createCommandTool("sweetpad.format.showLogs", "format_show_logs", "Show formatter logs", extensionContext);
  server.tool(formatShowLogs.toolName, "Show formatter logs", formatShowLogs.schema, formatShowLogs.implementation);

  // === SIMULATOR COMMANDS ===
  const simulatorsRefresh = createCommandTool("sweetpad.simulators.refresh", "simulators_refresh", "Refresh simulator list", extensionContext);
  server.tool(simulatorsRefresh.toolName, "Refresh simulator list", simulatorsRefresh.schema, simulatorsRefresh.implementation);

  const simulatorsOpenSimulator = createCommandTool("sweetpad.simulators.openSimulator", "simulators_open_simulator", "Open iOS Simulator", extensionContext);
  server.tool(simulatorsOpenSimulator.toolName, "Open iOS Simulator", simulatorsOpenSimulator.schema, simulatorsOpenSimulator.implementation);

  const simulatorsRemoveCache = createCommandTool("sweetpad.simulators.removeCache", "simulators_remove_cache", "Remove simulator cache", extensionContext);
  server.tool(simulatorsRemoveCache.toolName, "Remove simulator cache", simulatorsRemoveCache.schema, simulatorsRemoveCache.implementation);

  const simulatorsStart = createCommandTool("sweetpad.simulators.start", "simulators_start", "Start iOS Simulator", extensionContext);
  server.tool(simulatorsStart.toolName, "Start iOS Simulator", simulatorsStart.schema, simulatorsStart.implementation);

  const simulatorsStop = createCommandTool("sweetpad.simulators.stop", "simulators_stop", "Stop iOS Simulator", extensionContext);
  server.tool(simulatorsStop.toolName, "Stop iOS Simulator", simulatorsStop.schema, simulatorsStop.implementation);

  // === DEVICE COMMANDS ===
  const devicesRefresh = createCommandTool("sweetpad.devices.refresh", "devices_refresh", "Refresh device list", extensionContext);
  server.tool(devicesRefresh.toolName, "Refresh device list", devicesRefresh.schema, devicesRefresh.implementation);

  // === DESTINATION COMMANDS ===
  const destinationsSelect = createCommandTool("sweetpad.destinations.select", "destinations_select", "Select build destination", extensionContext);
  server.tool(destinationsSelect.toolName, "Select build destination", destinationsSelect.schema, destinationsSelect.implementation);

  const destinationsRemoveRecent = createCommandTool("sweetpad.destinations.removeRecent", "destinations_remove_recent", "Remove recent destination", extensionContext);
  server.tool(destinationsRemoveRecent.toolName, "Remove recent destination", destinationsRemoveRecent.schema, destinationsRemoveRecent.implementation);

  const destinationsSelectForTesting = createCommandTool("sweetpad.destinations.selectForTesting", "destinations_select_for_testing", "Select destination for testing", extensionContext);
  server.tool(destinationsSelectForTesting.toolName, "Select destination for testing", destinationsSelectForTesting.schema, destinationsSelectForTesting.implementation);

  // === TOOL COMMANDS ===
  const toolsInstall = createCommandTool("sweetpad.tools.install", "tools_install", "Install development tool", extensionContext);
  server.tool(toolsInstall.toolName, "Install development tool", toolsInstall.schema, toolsInstall.implementation);

  const toolsRefresh = createCommandTool("sweetpad.tools.refresh", "tools_refresh", "Refresh tools list", extensionContext);
  server.tool(toolsRefresh.toolName, "Refresh tools list", toolsRefresh.schema, toolsRefresh.implementation);

  const toolsDocumentation = createCommandTool("sweetpad.tools.documentation", "tools_documentation", "Open tool documentation", extensionContext);
  server.tool(toolsDocumentation.toolName, "Open tool documentation", toolsDocumentation.schema, toolsDocumentation.implementation);

  // === SYSTEM COMMANDS ===
  const systemResetSweetpadCache = createCommandTool("sweetpad.system.resetSweetpadCache", "system_reset_sweetpad_cache", "Reset SweetPad cache", extensionContext);
  server.tool(systemResetSweetpadCache.toolName, "Reset SweetPad cache", systemResetSweetpadCache.schema, systemResetSweetpadCache.implementation);

  const systemCreateIssueGeneric = createCommandTool("sweetpad.system.createIssue.generic", "system_create_issue_generic", "Create generic GitHub issue", extensionContext);
  server.tool(systemCreateIssueGeneric.toolName, "Create generic GitHub issue", systemCreateIssueGeneric.schema, systemCreateIssueGeneric.implementation);

  const systemCreateIssueNoSchemes = createCommandTool("sweetpad.system.createIssue.noSchemes", "system_create_issue_no_schemes", "Create GitHub issue for no schemes found", extensionContext);
  server.tool(systemCreateIssueNoSchemes.toolName, "Create GitHub issue for no schemes found", systemCreateIssueNoSchemes.schema, systemCreateIssueNoSchemes.implementation);

  const systemTestErrorReporting = createCommandTool("sweetpad.system.testErrorReporting", "system_test_error_reporting", "Test error reporting system", extensionContext);
  server.tool(systemTestErrorReporting.toolName, "Test error reporting system", systemTestErrorReporting.schema, systemTestErrorReporting.implementation);

  const systemOpenTerminalPanel = createCommandTool("sweetpad.system.openTerminalPanel", "system_open_terminal_panel", "Open terminal panel", extensionContext);
  server.tool(systemOpenTerminalPanel.toolName, "Open terminal panel", systemOpenTerminalPanel.schema, systemOpenTerminalPanel.implementation);

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