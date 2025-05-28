import * as vscode from 'vscode';
import { commonLogger } from './common/logger';
import { ExtensionContext } from './common/commands';
import { McpServer, ToolCallback } from "@modelcontextprotocol/sdk/server/mcp.js";
import { SSEServerTransport } from "@modelcontextprotocol/sdk/server/sse.js"; 
import { ZodRawShape, z } from 'zod';
import express, { Express, Request, Response } from 'express';
import { McpServerOptions, McpServerInstance, McpToolDefinition } from './types'; 
import { setupMetrics } from './metrics';
import { RequestHandlerExtra } from '@modelcontextprotocol/sdk/shared/protocol.js';
import http from 'http';
import { executeCommandSchema, executeCommandImplementation, ExecuteCommandArgs } from './tools/executeCommand'; 
import { CallToolResult } from '@modelcontextprotocol/sdk/types.js';

const SSE_ENDPOINT = '/sse';
const MESSAGES_ENDPOINT = '/messages';
const METRICS_ENDPOINT = '/metrics';

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

  server.tool(
      "execute_vscode_command", 
      "Executes a VS Code command and waits for task completion signal.",
      executeCommandSchema.shape,
      async (args: ExecuteCommandArgs, frameworkExtra: RequestHandlerExtra<any, any>): Promise<CallToolResult> => {
        return executeCommandImplementation(args, { extensionContext: extensionContext });
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