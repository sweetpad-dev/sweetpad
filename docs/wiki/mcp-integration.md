# Model Context Protocol (MCP) Integration

SweetPad includes built-in support for the [Model Context Protocol (MCP)](https://modelcontextprotocol.io/), enabling AI assistants to interact directly with your iOS development workflow through standardized tools.

## Overview

The MCP server in SweetPad provides a bridge between AI assistants and VS Code commands, allowing external tools to execute SweetPad functionality programmatically. This enables powerful workflows where AI assistants can build, test, format, and manage iOS projects on your behalf.

## Architecture

SweetPad's MCP implementation uses:
- **Transport**: HTTP with Server-Sent Events (SSE) 
- **Port**: 61337 (default)
- **Server Name**: "SweetPadCommandRunner"
- **Protocol Version**: Latest MCP specification

### System Architecture

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   MCP Client    │────│   SweetPad MCP   │────│   VS Code API   │
│   (Cursor)      │    │     Server       │    │   Commands      │
└─────────────────┘    └──────────────────┘    └─────────────────┘
         │                       │                       │
         │                       │                       │
    SSE + HTTP              Express Server           Command Bus
                                 │                       │
                          ┌──────┴──────┐        ┌──────┴──────┐
                          │  Execution  │        │    Task     │
                          │   Scope     │        │  Terminal   │
                          │ Management  │        │  System     │
                          └─────────────┘        └─────────────┘
                                 │                       │
                        ┌────────┴────────┐             │
                        │   Xcode Tools   │─────────────┘
                        │ (xcodebuild,    │
                        │  simulators,    │
                        │  devices, etc.) │
                        └─────────────────┘
```

### Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/sse` | GET | Establish SSE connection for real-time communication |
| `/messages` | POST | Handle MCP protocol messages |
| `/metrics` | GET | Prometheus metrics for monitoring |

## Available Tools

### `execute_vscode_command`

Executes any VS Code command and waits for task completion.

**Parameters:**
- `commandId` (string): The VS Code command ID to execute

**Examples:**
```json
{
  "tool": "execute_vscode_command",
  "arguments": {
    "commandId": "sweetpad.build.build"
  }
}
```

**Supported SweetPad Commands:**
- `sweetpad.build.build` - Build the current project
- `sweetpad.build.run` - Run the current project  
- `sweetpad.build.test` - Run tests
- `sweetpad.build.clean` - Clean build artifacts
- `sweetpad.format.run` - Format Swift code
- `sweetpad.simulators.start` - Start iOS Simulator
- `sweetpad.destinations.select` - Select build destination
- And many more...

## Client Configuration

### Cursor

Add this configuration to your Cursor MCP config file:

```json
{
  "mcpServers": {
    "sweetpad-mcp": {
      "url": "http://localhost:61337/SSE"
    }
  }
}
```

#### Rule

Add in your Cursor Rules: [sweetpad.server.mcp](cursor.rules/sweetpad.server.mcp)

**Configuration File Location:**
- **macOS**: `~/.cursor/mcp.json`

### Other MCP Clients

For HTTP-based MCP clients, use:
- **Base URL**: `http://localhost:61337`
- **SSE Endpoint**: `/sse`
- **Messages Endpoint**: `/messages`

## Server Lifecycle

The MCP server automatically:
1. **Starts** when the SweetPad extension activates
2. **Listens** on port 61337 for client connections
3. **Stops** when the extension deactivates or VS Code closes

## Monitoring & Metrics

Access Prometheus metrics at `http://localhost:61337/metrics` to monitor:
- Tool execution counts
- Response times
- Connection status
- Error rates

## Usage Examples

### Building a Project

```json
{
  "tool": "execute_vscode_command",
  "arguments": {
    "commandId": "sweetpad.build.build"
  }
}
```

### Running Tests

```json
{
  "tool": "execute_vscode_command", 
  "arguments": {
    "commandId": "sweetpad.build.test"
  }
}
```

### Formatting Code

```json
{
  "tool": "execute_vscode_command",
  "arguments": {
    "commandId": "sweetpad.format.run"
  }
}
```

## Security Considerations

- The MCP server only accepts connections from `localhost`
- All commands execute with the same permissions as VS Code
- Task completion is monitored with a 10-minute timeout
- No authentication is required for local connections

## Troubleshooting

### Connection Issues

1. **Check Port Availability**: Ensure port 61337 is not in use
2. **Verify Extension Status**: Confirm SweetPad extension is active
3. **Check Logs**: Look for MCP-related messages in VS Code output panel

### Common Errors

**"No transport found for sessionId"**
- Client connection was lost or timed out
- Reconnect by reestablishing the SSE connection

**"TIMEOUT after 600s"**
- Command took longer than 10 minutes
- Check VS Code for hung processes or dialogs

## Development

### Adding New Tools

To add custom MCP tools:

1. Define the tool schema:
```typescript
const myToolSchema = z.object({
  parameter: z.string().describe('Tool parameter')
});
```

2. Implement the tool function:
```typescript
const myToolImplementation = async (args, extra) => {
  // Tool logic here
  return { content: [{ type: "text", text: "Result" }] };
};
```

3. Register with the server:
```typescript
mcpInstance.registerTool({
  name: "my_tool",
  description: "My custom tool", 
  schema: myToolSchema.shape,
  implementation: myToolImplementation
});
```

### Extension Integration

The MCP server integrates with SweetPad's extension context and managers:
- Access to build manager for project operations
- Integration with destination manager for device/simulator control
- Connection to task completion events for proper synchronization

## API Reference

### McpServerOptions

```typescript
interface McpServerOptions {
  name: string;      // Server identifier
  version: string;   // Server version
  port?: number;     // Listen port (default: 8000)
}
```

### McpServerInstance

```typescript
interface McpServerInstance {
  app: Express;                               // Express server
  server: McpServer;                         // MCP server instance
  registerTool: (tool: McpToolDefinition) => void;  // Tool registration
  start: () => Promise<Express>;             // Start server
}
```

## Related Documentation

- [SweetPad Commands](commands.md)
- [Build System](build-system.md)
- [Testing Framework](testing.md)
- [Model Context Protocol Specification](https://modelcontextprotocol.io/) 