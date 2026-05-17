import { LineBuffer } from "../core/tasks/line-buffer";
import type { WireMessage } from "./types";

/**
 * Newline-delimited JSON framer. Feed raw socket chunks via `append`; each
 * complete line is parsed as a `WireMessage` and handed to the callback. Lines
 * that fail to parse are dropped and reported via `onError` so a malformed
 * payload doesn't kill the connection.
 */
export class MessageFramer {
  private buffer: LineBuffer;

  constructor(options: { onMessage: (message: WireMessage) => void; onError?: (line: string, error: unknown) => void }) {
    this.buffer = new LineBuffer({
      enabled: true,
      callback: (line) => {
        if (!line) return;
        try {
          const parsed = JSON.parse(line) as WireMessage;
          options.onMessage(parsed);
        } catch (error) {
          options.onError?.(line, error);
        }
      },
    });
  }

  append(chunk: string | Buffer): void {
    this.buffer.append(typeof chunk === "string" ? chunk : chunk.toString("utf8"));
  }

  flush(): void {
    this.buffer.flush();
  }
}

/** Serialise a single message to a single newline-terminated JSON line. */
export function encodeMessage(message: WireMessage): string {
  return `${JSON.stringify(message)}\n`;
}
