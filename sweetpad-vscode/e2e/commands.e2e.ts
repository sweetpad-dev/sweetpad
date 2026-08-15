import * as assert from "node:assert/strict";

import * as vscode from "vscode";

import { EXTENSION_ID, activate } from "./helpers";

/**
 * The contract between `package.json` and the code that registers commands.
 * Nothing else checks it: a command contributed but never registered shows up
 * in the palette and fails only when a user picks it, and a command registered
 * but never contributed is unreachable from the palette. Both are invisible to
 * the unit tests, which stub `vscode.commands` wholesale.
 */
suite("contributed commands", () => {
  let contributed: string[];
  let registered: Set<string>;

  suiteSetup(async () => {
    await activate();
    const extension = vscode.extensions.getExtension(EXTENSION_ID);
    contributed = (extension?.packageJSON.contributes.commands ?? []).map((c: { command: string }) => c.command);
    registered = new Set(await vscode.commands.getCommands(true));
  });

  test("package.json contributes at least one command", () => {
    assert.ok(contributed.length > 0, "no contributed commands found");
  });

  test("every contributed command is registered", () => {
    // Some commands only register when dev features are on, and dev features
    // key off `ExtensionMode.Development` — which a test host is not, it is
    // `ExtensionMode.Test`. Those same commands are the ones the manifest hides
    // behind a `sweetpad.devFeatures` menu clause, so read the exemption off the
    // manifest instead of hardcoding a list that would rot.
    const missing = contributed.filter((id) => !registered.has(id) && !devFeatureCommands().has(id));
    assert.deepEqual(missing, [], `contributed but not registered: ${missing.join(", ")}`);
  });

  /** Command ids the manifest shows only when the `sweetpad.devFeatures` context is set. */
  function devFeatureCommands(): Set<string> {
    const extension = vscode.extensions.getExtension(EXTENSION_ID);
    const menus = (extension?.packageJSON.contributes.menus ?? {}) as Record<
      string,
      { command?: string; when?: string }[]
    >;
    return new Set(
      Object.values(menus)
        .flat()
        .filter((entry) => entry.when?.includes("sweetpad.devFeatures") && entry.command)
        .map((entry) => entry.command as string),
    );
  }

  test("every registered sweetpad command is contributed", () => {
    const extension = vscode.extensions.getExtension(EXTENSION_ID);
    // VS Code synthesizes a handful of commands per contributed view — they are
    // registered without appearing in `contributes.commands`, and flagging them
    // would just teach the reader to ignore this assertion. Derive them from the
    // views themselves rather than pattern-matching, so a genuine orphan that
    // happens to end in `.focus` is still caught.
    const viewsByContainer = (extension?.packageJSON.contributes.views ?? {}) as Record<string, { id: string }[]>;
    const views: string[] = Object.values(viewsByContainer)
      .flat()
      .map((view) => view.id);
    const synthesized = new Set(
      views.flatMap((id) =>
        ["open", "focus", "resetViewLocation", "toggleVisibility", "removeView"].map((action) => `${id}.${action}`),
      ),
    );

    // Commands the extension registers for internal wiring rather than the
    // palette are exempt by convention: they are namespaced under `_`.
    const orphans = [...registered].filter(
      (id) =>
        id.startsWith("sweetpad.") && !id.startsWith("sweetpad._") && !contributed.includes(id) && !synthesized.has(id),
    );
    assert.deepEqual(orphans, [], `registered but not contributed: ${orphans.join(", ")}`);
  });
});
