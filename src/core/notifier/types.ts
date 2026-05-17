/**
 * Surfaces user-facing notifications (info banners, warnings, errors) raised
 * by the engine. The host decides whether to render them as a VS Code toast,
 * a CLI stderr line, etc.
 */
export interface Notifier {
  info(message: string): void;
  warn(message: string): void;
  error(message: string): void;
}

export const noopNotifier: Notifier = {
  info() {},
  warn() {},
  error() {},
};
