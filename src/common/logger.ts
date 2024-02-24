import * as vscode from "vscode";

interface Context {
  message?: never;
  type?: never;
  time?: never;
  [key: string]: any;
}

interface Message {
  message: string;
  type: "info" | "error";
  time: string;
  [key: string]: any;
}

/**
 * Logger is a wrapper around vscode.OutputChannel that provides a simple way to
 * log messages to the SweetPad output channel. Messages are formatted as JSON
 * and include a timestamp, message type, and any additional context.
 */
export class Logger {
  private outputChannel: vscode.OutputChannel;

  constructor(options: { name: string }) {
    this.outputChannel = vscode.window.createOutputChannel(`SweetPad: ${options.name}`);
  }

  private format(data: Message) {
    return JSON.stringify(data, null, 2);
  }

  private addMessage(data: Message) {
    const formatted = this.format(data);
    this.outputChannel.appendLine(formatted);
  }

  private getNow() {
    return new Date().toISOString();
  }

  log(message: string, context: Context) {
    this.addMessage({
      message: message,
      type: "info",
      time: this.getNow(),
      ...context,
    });
  }

  error(message: string, context: Context) {
    this.addMessage({
      message: message,
      type: "error",
      time: this.getNow(),
      ...context,
    });
  }

  show() {
    this.outputChannel.show();
  }
}

export const commonLogger = new Logger({ name: "Common" });
