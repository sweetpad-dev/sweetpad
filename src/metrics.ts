import { Counter, Histogram, Registry } from 'prom-client';
import { commonLogger } from './common/logger'; // Use commonLogger
import { RequestHandlerExtra } from '@modelcontextprotocol/sdk/shared/protocol.js';
import { CallToolResult } from '@modelcontextprotocol/sdk/types.js';
import { ZodRawShape, ZodTypeAny, z } from 'zod';

// Create a registry for MCP-specific metrics
const registry = new Registry();

// Define the metrics
export const TOOL_CALLS = new Counter({
  name: 'mcp_tool_calls_total',
  help: 'Number of tool calls',
  labelNames: ['tool_name', 'status'],
  registers: [registry],
});

export const TOOL_LATENCY = new Histogram({
  name: 'mcp_tool_latency_seconds',
  help: 'Tool execution latency in seconds',
  labelNames: ['tool_name'],
  registers: [registry],
});

/**
 * Setup metrics for the MCP server
 * @returns Registry - Prometheus metrics registry
 */
export function setupMetrics(): Registry {
  return registry;
}

/**
 * Get the MCP metrics registry
 * @returns Registry - Prometheus metrics registry
 */
export function getMetrics(): Registry {
  return registry;
}

/**
 * Reset metrics (useful for testing)
 */
export function resetMetrics(): void {
  registry.resetMetrics();
}

/**
 * Wrap a tool function with metric collection
 *
 * @param toolName - Name of the tool
 * @param fn - Tool implementation function
 * @returns Wrapped function with metrics
 */
export function withMetrics<S extends ZodRawShape>(
  toolName: string,
  fn: (
    args: z.objectOutputType<S, ZodTypeAny>,
    extra: RequestHandlerExtra<any, any> 
  ) => Promise<CallToolResult> | CallToolResult
): (args: z.objectOutputType<S, ZodTypeAny>, extra: RequestHandlerExtra<any, any>) => Promise<CallToolResult> {
  return async (
    args: z.objectOutputType<S, ZodTypeAny>,
    extra: RequestHandlerExtra<any, any>
  ): Promise<CallToolResult> => {
    const start = process.hrtime.bigint();
    try {
      const result = await Promise.resolve(fn(args, extra));
      // Using commonLogger here instead of the internal one
      commonLogger.log(
        `Tool call ${toolName} with args ${JSON.stringify(args)} succeeded`
      );
      TOOL_CALLS.labels(toolName, 'success').inc();
      return result;
    } catch (error) {
      TOOL_CALLS.labels(toolName, 'error').inc();
      commonLogger.error(`Tool execution failed: ${toolName}`, { error, toolName }); 
      throw error;
    } finally {
      const end = process.hrtime.bigint();
      const duration = Number(end - start) / 1_000_000_000;
      TOOL_LATENCY.labels(toolName).observe(duration);
    }
  }
} 