/**
 * Surfaces progress messages from long-running engine operations.
 *
 * - VS Code adapter: writes to a status bar item.
 * - CLI adapter: prints to stderr as the operation streams.
 */
export interface ProgressReporter {
  updateText(text: string): void;
}

export const noopProgressReporter: ProgressReporter = {
  updateText() {},
};
