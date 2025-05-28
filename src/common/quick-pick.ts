import * as vscode from "vscode";

export type QuickPickItemRow<T> = vscode.QuickPickItem & { context: T };
export type QuickPickItemSeparator = vscode.QuickPickItem & { kind: vscode.QuickPickItemKind.Separator };
export type QuickPickItem<T> = QuickPickItemRow<T> | QuickPickItemSeparator;

export class QuickPickCancelledError extends Error {}

/**
 * Shows a quick pick dialog with the given options.
 * @param options - The options for the quick pick dialog.
 * @returns A promise that resolves with the selected item label.
 */
export async function showQuickPick<T>(options: {
  title: string;
  items: QuickPickItem<T>[];
}): Promise<QuickPickItemRow<T>> {
  const pick = vscode.window.createQuickPick<QuickPickItem<T>>();

  pick.items = options.items;
  pick.title = options.title;
  pick.placeholder = options.title;

  pick.show();

  return new Promise((resolve, reject) => {
    let isAccepted = false;
    pick.onDidAccept(() => {
      const selected = pick.selectedItems[0];
      isAccepted = true;

      pick.dispose();
      // I'm not sure if it's possible to select a separator, but just in case and to please the TypeScript checker
      if (!selected || selected?.kind === vscode.QuickPickItemKind.Separator) {
        reject(new Error("No item selected"));
      } else {
        resolve(selected);
      }
    });

    pick.onDidHide(() => {
      if (!isAccepted) {
        pick.dispose();
        // When the quick pick is cancled by the user (e.g. by pressing Escape), the promise is rejected
        // with a QuickPickCancelledError to silently cancel the command in which the quick pick was shown.
        reject(new QuickPickCancelledError());
      }
    });
  });
}

export async function showInputBox(options: {
  title: string;
  value?: string;
}): Promise<string | undefined> {
  return vscode.window.showInputBox({
    title: options.title,
    value: options.value,
  });
}
