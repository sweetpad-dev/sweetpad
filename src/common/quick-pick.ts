import * as vscode from "vscode";

interface QuickPickItem<T> extends vscode.QuickPickItem {
  context: T;
}

/**
 * Shows a quick pick dialog with the given options.
 * @param options - The options for the quick pick dialog.
 * @returns A promise that resolves with the selected item label.
 */
export async function showQuickPick<T>(options: {
  title: string;
  items: QuickPickItem<T>[];
}): Promise<QuickPickItem<T>> {
  const pick = vscode.window.createQuickPick<QuickPickItem<T>>();

  pick.items = options.items;
  pick.title = options.title;
  pick.placeholder = options.title;

  pick.show();

  return new Promise((resolve) => {
    pick.onDidAccept(() => {
      resolve(pick.selectedItems[0]);
      pick.dispose();
    });
  });
}
