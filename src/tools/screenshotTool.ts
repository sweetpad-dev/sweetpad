import { z } from 'zod';
import { CallToolResult } from '@modelcontextprotocol/sdk/types.js';
import { RequestHandlerExtra } from '@modelcontextprotocol/sdk/shared/protocol.js';
import { ExtensionContext } from '../common/commands';
import { commonLogger } from '../common/logger';
import { exec } from '../common/exec';
import * as fs from 'fs/promises';
import * as path from 'path';
import { prepareStoragePath } from '../build/utils';

export const addScreenshotContextSchema = z.object({
  simulatorName: z.string(),
  simulatorUdid: z.string(),
  filename: z.string(),
  path: z.string(),
  base64: z.string(),
  timestamp: z.string(),
});

export type AddScreenshotContextArgs = z.infer<typeof addScreenshotContextSchema>;

export type ScreenshotToolExtra = {
  extensionContext: ExtensionContext;
};

// Store screenshot context in memory (could be persisted if needed)
const screenshotContext: Map<string, AddScreenshotContextArgs> = new Map();

export async function addScreenshotContextImplementation(
  args: AddScreenshotContextArgs,
  extra: ScreenshotToolExtra
): Promise<CallToolResult> {
  try {
    commonLogger.log('Adding screenshot to AI context', {
      simulatorName: args.simulatorName,
      filename: args.filename,
      timestamp: args.timestamp
    });

    // Store screenshot data for AI context
    const contextKey = `${args.simulatorUdid}-${args.timestamp}`;
    screenshotContext.set(contextKey, args);

    // Clean up old screenshots (keep only last 5)
    const entries = Array.from(screenshotContext.entries());
    if (entries.length > 5) {
      const toRemove = entries.slice(0, entries.length - 5);
      toRemove.forEach(([key]) => screenshotContext.delete(key));
    }

    // Return image directly to AI as context - following MCP pattern
    return {
      content: [
        {
          type: "image",
          data: args.base64,
          mimeType: "image/png",
        },
        {
          type: 'text',
          text: `iOS Simulator screenshot from ${args.simulatorName} captured at ${args.timestamp}. This shows the current UI state of the running iOS app for analysis and debugging.`
        }
      ],
    };
  } catch (error) {
    commonLogger.error('Error adding screenshot to context', { error, args });
    return {
      content: [
        {
          type: 'text',
          text: `Failed to add screenshot to AI context: ${error instanceof Error ? error.message : String(error)}`
        }
      ],
      isError: true,
    };
  }
}

export function getScreenshotContext(): AddScreenshotContextArgs[] {
  return Array.from(screenshotContext.values());
}

export function clearScreenshotContext(): void {
  screenshotContext.clear();
}

// Direct screenshot tool - takes screenshot and returns as context
export const takeScreenshotSchema = z.object({
  simulatorUdid: z.string().optional(),
});

export type TakeScreenshotArgs = z.infer<typeof takeScreenshotSchema>;

export async function takeScreenshotImplementation(
  args: TakeScreenshotArgs,
  extra: ScreenshotToolExtra
): Promise<CallToolResult> {
  try {
    // Get running simulators
    const simulatorsOutput = await exec({
      command: "xcrun",
      args: ["simctl", "list", "--json", "devices"]
    });
    
    const devices = JSON.parse(simulatorsOutput).devices as Record<string, any[]>;
    const bootedSimulators = Object.entries(devices)
      .flatMap(([runtime, sims]) => 
        (sims as any[]).filter(sim => sim.state === "Booted").map(sim => ({...sim, runtime}))
      );

    if (bootedSimulators.length === 0) {
      return {
        content: [
          {
            type: 'text',
            text: 'No running simulators found. Please start a simulator first.'
          }
        ],
      };
    }

    // Use specified simulator or first booted one
    const targetSim = args.simulatorUdid 
      ? bootedSimulators.find(sim => sim.udid === args.simulatorUdid)
      : bootedSimulators[0];

    if (!targetSim) {
      return {
        content: [
          {
            type: 'text',
            text: `Simulator ${args.simulatorUdid} not found or not running.`
          }
        ],
      };
    }

    // Generate screenshot path
    const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
    const tempDir = await prepareStoragePath(extra.extensionContext);
    const screenshotPath = path.join(tempDir, `screenshot-${timestamp}.png`);

    // Take screenshot
    await exec({
      command: "xcrun",
      args: ["simctl", "io", targetSim.udid, "screenshot", screenshotPath]
    });

    // Convert to base64
    const imageBuffer = await fs.readFile(screenshotPath);
    const base64Image = imageBuffer.toString('base64');

    commonLogger.log('Screenshot taken for MCP', {
      simulator: targetSim.name,
      size: imageBuffer.length,
      path: screenshotPath
    });

    // Return image directly as context - exact MCP pattern
    return {
      content: [
        {
          type: "image",
          data: base64Image,
          mimeType: "image/png",
        }
      ],
    };

  } catch (error) {
    commonLogger.error('Error taking screenshot', { error });
    return {
      content: [
        {
          type: 'text',
          text: `Failed to take screenshot: ${error instanceof Error ? error.message : String(error)}`
        }
      ],
      isError: true,
    };
  }
}


