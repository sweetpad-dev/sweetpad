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

  const buildBuild = createCommandTool("sweetpad.build.build", "build_ios_project", "Build the iOS project without running", extensionContext);
  server.tool(buildBuild.toolName, "Build the iOS project without running", buildBuild.schema, buildBuild.implementation);

  const buildClean = createCommandTool("sweetpad.build.clean", "clean_build_artifacts", "Clean iOS build artifacts", extensionContext);
  server.tool(buildClean.toolName, "Clean iOS build artifacts", buildClean.schema, buildClean.implementation);

  const buildTest = createCommandTool("sweetpad.build.test", "run_unit_tests", "Run unit tests", extensionContext);
  server.tool(buildTest.toolName, "Run unit tests", buildTest.schema, buildTest.implementation);

  const buildResolveDependencies = createCommandTool("sweetpad.build.resolveDependencies", "resolve_swift_packages", "Resolve Swift Package Manager dependencies", extensionContext);
  server.tool(buildResolveDependencies.toolName, "Resolve Swift Package Manager dependencies", buildResolveDependencies.schema, buildResolveDependencies.implementation);

  const buildSelectXcodeWorkspace = createCommandTool("sweetpad.build.selectXcodeWorkspace", "select_xcode_workspace", "Select Xcode workspace file", extensionContext);
  server.tool(buildSelectXcodeWorkspace.toolName, "Select Xcode workspace file", buildSelectXcodeWorkspace.schema, buildSelectXcodeWorkspace.implementation);

  const buildSetDefaultScheme = createCommandTool("sweetpad.build.setDefaultScheme", "set_build_scheme", "Set default build scheme", extensionContext);
  server.tool(buildSetDefaultScheme.toolName, "Set default build scheme", buildSetDefaultScheme.schema, buildSetDefaultScheme.implementation);

  const buildSelectConfiguration = createCommandTool("sweetpad.build.selectConfiguration", "select_build_configuration", "Select build configuration (Debug/Release)", extensionContext);
  server.tool(buildSelectConfiguration.toolName, "Select build configuration (Debug/Release)", buildSelectConfiguration.schema, buildSelectConfiguration.implementation);

  const buildPeripheryScan = createCommandTool("sweetpad.build.peripheryScan", "scan_unused_code", "Run Periphery scan to find unused Swift code", extensionContext);
  server.tool(buildPeripheryScan.toolName, "Run Periphery scan to find unused Swift code", buildPeripheryScan.schema, buildPeripheryScan.implementation);

  const buildBuildAndPeripheryScan = createCommandTool("sweetpad.build.buildAndPeripheryScan", "build_and_scan_unused_code", "Build project and run Periphery scan", extensionContext);
  server.tool(buildBuildAndPeripheryScan.toolName, "Build project and run Periphery scan", buildBuildAndPeripheryScan.schema, buildBuildAndPeripheryScan.implementation);

  // === DESTINATION COMMANDS ===
  const destinationsSelect = createCommandTool("sweetpad.destinations.select", "select_build_destination", "Select build destination (device or simulator)", extensionContext);
  server.tool(destinationsSelect.toolName, "Select build destination (device or simulator)", destinationsSelect.schema, destinationsSelect.implementation);

  // === SCREENSHOT COMMANDS ===
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