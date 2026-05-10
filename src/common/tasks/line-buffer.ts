/**
 * Collect stdout or stderr output and send it line by line to the callback
 */
export class LineBuffer {
  public buffer = "";
  public enabled = true;
  public callback: (line: string) => void;

  constructor(options: { enabled: boolean; callback: (line: string) => void }) {
    this.enabled = options.enabled;
    this.callback = options.callback;
  }

  append(data: string): void {
    if (!this.enabled) return;

    this.buffer += data;

    const lines = this.buffer.split("\n");

    // last line can be not finished yet, so we need to keep it and send to callback later
    this.buffer = lines.pop() ?? "";

    // send all lines in buffer to callback, except last one
    for (const line of lines) {
      this.callback(line);
    }
  }

  flush(): void {
    if (!this.enabled) return;

    if (this.buffer) {
      this.callback(this.buffer);
      this.buffer = "";
    }
  }
}
