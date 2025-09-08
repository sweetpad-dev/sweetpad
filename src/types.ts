import { Express } from "express";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
// McpServerToolImplementation is likely inferred or part of McpServer types
// import { McpServerToolImplementation } from '@modelcontextprotocol/sdk/server/index.js';
import { ZodRawShape, ZodTypeAny, z } from "zod";
import { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { RequestHandlerExtra } from "@modelcontextprotocol/sdk/shared/protocol.js";
import { ToolCallback } from "@modelcontextprotocol/sdk/server/mcp.js";

/**
 * Options for creating an MCP server
 */
export interface McpServerOptions {
  /**
   * Name of the MCP server
   */
  name: string;

  /**
   * Version of the MCP server
   */
  version: string;

  /**
   * Port to listen on
   * @default 8000
   */
  port?: number;
}

/**
 * Return type for createMcpServer function
 */
export interface McpServerInstance {
  /**
   * Express app instance
   */
  app: Express;

  /**
   * McpServer instance from the SDK
   */
  server: McpServer;

  /**
   * Register a tool with the server
   */
  registerTool: <T extends ZodRawShape>(tool: McpToolDefinition<T>) => void;

  /**
   * Start the server
   */
  start: () => Promise<Express>;
}

/**
 * Compatible with SDK Server Request interface
 */
export interface McpRequest {
  method: string;
  params?: {
    [key: string]: unknown;
    _meta?: {
      [key: string]: unknown;
      progressToken?: string | number;
    };
  };
}

/**
 * Compatible with SDK Server Response interface
 */
export interface McpResponse {
  method: string;
  params?: {
    [key: string]: unknown;
    _meta?: {
      [key: string]: unknown;
    };
  };
}

/**
 * Function type for tool implementations
 */
export type ToolImplementation = (
  args: Record<string, unknown>,
  extra: RequestHandlerExtra<any, any>,
) => Promise<CallToolResult> | CallToolResult;

/**
 * Definition of an MCP tool
 */
export interface McpToolDefinition<T extends ZodRawShape> {
  /**
   * Name of the tool
   */
  name: string;

  /**
   * Description of the tool
   */
  description: string;

  /**
   * Zod schema for tool parameters
   */
  schema: T;

  /**
   * Tool implementation function
   */
  implementation: ToolCallback<T>;
}
