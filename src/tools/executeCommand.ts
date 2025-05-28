import * as vscode from 'vscode'; // Needed for executeCommand
import { z } from 'zod';
import { commonLogger } from '../common/logger'; 
import { ExtensionContext } from '../common/commands';
import { CallToolResult } from '@modelcontextprotocol/sdk/types.js';

// Schema only needs commandId
export const executeCommandSchema = z.object({
  commandId: z.string().describe('The VS Code command ID to execute.'),
});

export type ExecuteCommandArgs = z.infer<typeof executeCommandSchema>;

export type ExecuteCommandExtra = {
    extensionContext: ExtensionContext;
}

export const executeCommandImplementation = async (
    args: ExecuteCommandArgs, 
    extra: ExecuteCommandExtra
): Promise<CallToolResult> => {
    const { commandId } = args;
    const timeoutSeconds = 600; 
    
    let eventListener: vscode.Disposable | undefined;
    const waitForCompletionPromise = new Promise<"completed">((resolve) => {
        eventListener = extra.extensionContext.simpleTaskCompletionEmitter.event(() => {
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
        return { content: [{ type: 'text', text: `Read the output file for ${commandId} to determine next steps. File: ${extra.extensionContext.UI_LOG_PATH()}` }], isError: false };
    }
};

// Remove default export if createToolFunction is not used 