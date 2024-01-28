#!/usr/bin/env node

// BSP server for apple sourcekit-lsp
// Work in progress, not usable yet.

import fs from "fs/promises";

process.stdin.setEncoding("utf8");

const OPTIONS = {
  options: [],
  workingDirectory: "",
};

/**
 * Write logs to stderr so they don't interfere with the protocol
 */
class Logger {
  log(message: string, obj?: any) {
    process.stderr.write(`Sweetpad BS: ${message} - ${JSON.stringify(obj, null, 2)}\n`);
  }
}

const logger = new Logger();

class MessageProcessor {
  private buffer = "";
  private expectedContentLength = -1;

  public async *read(): AsyncGenerator<any> {
    // message format:
    // Content-Length: <length>\r\n\r\n<json>
    // multiple messages can be sent in a single chunk

    for await (const chunk of process.stdin) {
      this.buffer += chunk;

      if (this.expectedContentLength === -1) {
        if (!chunk.includes("\r\n\r\n")) {
          // we haven't received the full header yet, read more
          continue;
        }

        // we find the end of the header
        const [header, body] = this.buffer.split("\r\n\r\n", 2);
        const contentLengthRaw = header.split("Content-Length:")[1];
        const contentLength = parseInt(contentLengthRaw.trim(), 10);
        this.expectedContentLength = contentLength;
        this.buffer = body;
      }

      if (this.buffer.length < this.expectedContentLength) {
        // we haven't received the full body yet
        continue;
      }

      // we have received the full body
      const raw = this.buffer.slice(0, this.expectedContentLength);
      this.buffer = this.buffer.slice(this.expectedContentLength);
      this.expectedContentLength = -1;
      yield JSON.parse(raw);
    }
  }

  async write(obj: any) {
    const raw = JSON.stringify(obj);
    process.stdout.write(`Content-Length: ${raw.length}\r\n\r\n${raw}`);
  }
}

class Server {
  private processor = new MessageProcessor();

  async getOptions(options: { filename: string }) {
    const compileFile = "...";
    const compileJSON: any[] = JSON.parse(await fs.readFile(compileFile, "utf8"));

    for (const item of compileJSON) {
      if (!item.command) {
        continue;
      }
    }
    return OPTIONS;
  }

  async dispatchMessage(message: any): Promise<void> {
    if (message.method === "build/initialize") {
      await this.send({
        jsonrpc: "2.0",
        id: message.id,
        result: {
          displayName: "xcode build server",
          version: "0.1",
          bspVersion: "2.0",
          rootUri: "...",
          capabilities: { languageIds: ["c", "cpp", "objective-c", "objective-cpp", "swift"] },
          data: {
            indexDatabasePath: "...",
            indexStorePath: "...",
          },
        },
      });
    } else if (message.method === "build/initialized") {
      // noop, nothing to do here
    } else if (message.method === "workspace/buildTargets") {
      await this.send({
        jsonrpc: "2.0",
        id: message.id,
        result: {
          items: [],
        },
      });
    } else if (message.method === "buildTarget/sources") {
      await this.send({
        jsonrpc: "2.0",
        id: message.id,
        result: {
          sources: [],
        },
      });
    } else if (message.method === "textDocument/registerForChanges") {
      await this.send({
        jsonrpc: "2.0",
        id: message.id,
        result: null,
      });

      const options = await this.getOptions({
        filename: message.params.uri,
      });

      await this.send({
        jsonrpc: "2.0",
        method: "build/sourceKitOptionsChanged",
        params: {
          uri: message.params.uri,
          updatedOptions: options,
        },
      });
    } else if (message.method === "textDocument/sourceKitOptions") {
      await this.send({
        jsonrpc: "2.0",
        id: message.id,
        result: OPTIONS,
      });
    } else {
      await this.send({
        jsonrpc: "2.0",
        id: message.id,
        error: {
          code: -32601,
          message: `Method ${message.method} not implemented`,
        },
      });
    }
  }

  /**
   * Send message to the client (SourceKit-LSP) and log it
   */
  async send(message: any) {
    logger.log("Response", message);
    await this.processor.write(message);
  }

  async serve() {
    const processor = new MessageProcessor();
    for await (const message of processor.read()) {
      logger.log("Request", message);
      await this.dispatchMessage(message);
    }
  }
}

const server = new Server();
server.serve().catch((err) => console.error("An error occurred:", err));
