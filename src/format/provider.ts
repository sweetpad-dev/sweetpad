import * as vscode from "vscode";
import { registerFormatProvider, registerRangeFormatProvider, SwiftFormattingProvider } from "./formatter.js";

export function createFormatProvider(): vscode.Disposable {
  const provider = new SwiftFormattingProvider();
  
  return vscode.Disposable.from(
    registerFormatProvider(provider),
    registerRangeFormatProvider(provider)
  );
} 