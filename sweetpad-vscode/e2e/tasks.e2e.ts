import * as assert from "node:assert/strict";

import * as vscode from "vscode";

import { EXTENSION_ID, activate } from "./helpers";

/**
 * The task surface users write into `tasks.json`. The manifest advertises a set
 * of actions; the provider has to offer a task for each one, or a `tasks.json`
 * a user copied from the docs silently does nothing.
 *
 * Says nothing about what a task runs — only that every advertised action is
 * reachable, which stays true however the work is carried out.
 */
suite("task provider", () => {
  suiteSetup(async () => {
    await activate();
  });

  test("offers no action the manifest does not document", async () => {
    const extension = vscode.extensions.getExtension(EXTENSION_ID);
    const definitions = (extension?.packageJSON.contributes.taskDefinitions ?? []) as {
      type: string;
      properties?: { action?: { enum?: string[] } };
    }[];
    const advertised = definitions.find((d) => d.type === "sweetpad")?.properties?.action?.enum ?? [];
    const offered = (await vscode.tasks.fetchTasks({ type: "sweetpad" })).map(
      (task) => task.definition.action as string,
    );

    // This is the direction that is unambiguously a contract. The reverse — an
    // advertised action the provider doesn't *offer* — is only a discoverability
    // gap: `resolveTask` still runs it from a hand-written `tasks.json`.
    const undocumented = offered.filter((action) => !advertised.includes(action));
    assert.deepEqual(
      undocumented,
      [],
      `offered by the provider but undocumented in package.json: ${undocumented.join(", ")}`,
    );
  });

  test("offers the actions a user reaches from the task list", async () => {
    const extension = vscode.extensions.getExtension(EXTENSION_ID);
    const definitions = (extension?.packageJSON.contributes.taskDefinitions ?? []) as {
      type: string;
      properties?: { action?: { enum?: string[] } };
    }[];
    const sweetpad = definitions.find((definition) => definition.type === "sweetpad");
    assert.ok(sweetpad, "the manifest contributes no `sweetpad` task type");
    assert.ok((sweetpad.properties?.action?.enum ?? []).length > 0, "the `action` property advertises no values");

    const offered = new Set(
      (await vscode.tasks.fetchTasks({ type: "sweetpad" })).map((task) => task.definition.action as string),
    );
    // The everyday actions, which have to be reachable without hand-writing a
    // `tasks.json`. Deliberately not the full advertised set: `test` is
    // advertised and deliberately not offered, and pinning the whole list here
    // would turn every future addition into a failing test.
    for (const action of ["build", "launch", "clean"]) {
      assert.ok(offered.has(action), `the task list offers no "${action}" task; offers: ${[...offered].join(", ")}`);
    }
  });
});
