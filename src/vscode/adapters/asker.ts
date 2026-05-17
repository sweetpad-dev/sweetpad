import * as vscode from "vscode";

import { type PickItem, type PickItemRow, UserPickCancelledError, type UserAsker } from "../../core/asker/types";

type QuickPickItemBase = vscode.QuickPickItem & { srcIndex?: number };

/**
 * VS Code-backed UserAsker. Translates core `PickItem<T>` items to
 * `vscode.QuickPickItem` and surfaces cancellation via `UserPickCancelledError`.
 */
export class VsCodeAsker implements UserAsker {
  async pick<T>(options: { title: string; items: PickItem<T>[] }): Promise<PickItemRow<T>> {
    const vscodeItems: QuickPickItemBase[] = options.items.map((item, index) => {
      if ("kind" in item && item.kind === "separator") {
        return {
          label: item.label,
          kind: vscode.QuickPickItemKind.Separator,
        };
      }
      const row = item as PickItemRow<T>;
      return {
        label: row.label,
        description: row.description,
        detail: row.detail,
        iconPath: row.iconId ? new vscode.ThemeIcon(row.iconId) : undefined,
        srcIndex: index,
      };
    });

    return await new Promise<PickItemRow<T>>((resolve, reject) => {
      const pick = vscode.window.createQuickPick<QuickPickItemBase>();
      pick.items = vscodeItems;
      pick.title = options.title;
      pick.placeholder = options.title;

      // settle() guards against the double-fire that happens when we call
      // `pick.dispose()` from inside an `onDidHide` handler — dispose re-fires
      // `onDidHide`, which would otherwise reject a second time on an already-
      // settled promise (showing an empty Error in the user's logs).
      let settled = false;
      const settle = (fn: () => void) => {
        if (settled) return;
        settled = true;
        fn();
      };

      pick.onDidAccept(() => {
        const selected = pick.selectedItems[0];
        pick.dispose();
        settle(() => {
          if (!selected || selected.kind === vscode.QuickPickItemKind.Separator) {
            reject(new Error("No item selected"));
            return;
          }
          const idx = selected.srcIndex;
          if (idx === undefined) {
            reject(new Error("Internal: lost pick index"));
            return;
          }
          const sourceItem = options.items[idx];
          if (!sourceItem || "kind" in sourceItem) {
            reject(new Error("Internal: separator selected"));
            return;
          }
          resolve(sourceItem);
        });
      });

      pick.onDidHide(() => {
        // The user cancelled (Esc / click-outside). Surface a typed error so
        // `registerCommand` can swallow it silently. Do NOT call `pick.dispose()`
        // here — onDidHide fires *from* dispose, and re-disposing causes another
        // hide event that re-enters this handler.
        settle(() => reject(new UserPickCancelledError()));
      });
    });
  }

  async input(options: { title: string; value?: string }): Promise<string | undefined> {
    return await vscode.window.showInputBox({
      title: options.title,
      value: options.value,
    });
  }
}
