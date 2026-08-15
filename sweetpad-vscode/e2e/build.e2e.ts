import * as assert from "node:assert/strict";
import { readFileSync, rmSync, writeFileSync } from "node:fs";

import * as vscode from "vscode";

import { activate, derivedDataPath, findFile, findFiles, seedBuildSelection, waitFor, workspacePath } from "./helpers";

/**
 * Building produces a product, cleaning removes it, the settings that steer it
 * are honoured, and a build that cannot compile does not pretend otherwise.
 *
 * Stated as outcomes — "a product for this target appears under the directory we
 * nominated", "the two configurations do not write to the same place" — rather
 * than as paths or arguments, because where output lands and how the work is
 * carried out are the build system's business, and this suite has to hold
 * across a change of build system.
 *
 * The fixture is a macOS static library: about a second per warm build, and no
 * simulator, signing, or device.
 */
suite("build", () => {
  /** The library a target named `ObjCHeaders` produces. A fact about the fixture, not the pipeline. */
  const PRODUCT = "libObjCHeaders.a";

  /**
   * The shared output directory every suite builds into. Deleting the whole
   * directory to get a clean precondition would force another cold build, which
   * costs more than every other test here combined — so remove just the product
   * instead. "The build put it back" is the same assertion, warm.
   */
  const output = () => derivedDataPath();
  const source = () => workspacePath("widget.m");
  const config = () => vscode.workspace.getConfiguration("sweetpad");

  let pristine: string;

  suiteSetup(async () => {
    await activate();
    await seedBuildSelection();
    pristine = readFileSync(source(), "utf8");
    for (const stale of findFiles(output(), PRODUCT)) {
      rmSync(stale, { force: true });
    }
  });

  suiteTeardown(async () => {
    // Other suites read this file expecting it to compile.
    writeFileSync(source(), pristine, "utf8");
    await config().update("build.configuration", "Debug", vscode.ConfigurationTarget.Workspace);
  });

  test("produces the target's product under the configured output directory", async () => {
    assert.equal(
      findFile(output(), PRODUCT),
      undefined,
      "product existed before the build — the assertion would prove nothing",
    );

    // The command layer reports failures as notifications and resolves anyway,
    // so a silent no-op and a real build look identical from here. Recording the
    // task lifecycle turns "no product" into a diagnosis: no task at all means
    // it stopped before running; a non-zero exit means the build itself failed.
    const trace: string[] = [];
    const subscriptions = [
      vscode.tasks.onDidStartTask((e) => trace.push(`started ${e.execution.task.name}`)),
      vscode.tasks.onDidEndTaskProcess((e) => trace.push(`exited ${e.execution.task.name} code=${e.exitCode}`)),
    ];

    try {
      await vscode.commands.executeCommand("sweetpad.build.build");
      await waitFor(() => findFile(output(), PRODUCT) !== undefined, {
        timeout: 180_000,
        message: `no ${PRODUCT} under ${output()}\ntask activity: ${
          trace.length ? trace.join(" | ") : "(no task ever started)"
        }`,
      });
    } finally {
      for (const subscription of subscriptions) {
        subscription.dispose();
      }
    }
  });

  test("building again is safe and leaves the product in place", async () => {
    // Incremental builds are the common case, and "the second build blew up" is
    // a regression a single-shot test would never see.
    await vscode.commands.executeCommand("sweetpad.build.build");
    await waitFor(() => findFile(output(), PRODUCT) !== undefined, {
      timeout: 180_000,
      message: "the product disappeared after a second build",
    });
  });

  test("each configuration gets its own output", async () => {
    // Says only that the setting *has an effect* and that the two builds do not
    // overwrite each other — never what the directories are called, which is
    // exactly the sort of layout detail that need not survive the migration.
    const debugProduct = findFile(output(), PRODUCT);
    assert.ok(debugProduct, "expected the Debug build's product to still be there");

    await config().update("build.configuration", "Release", vscode.ConfigurationTarget.Workspace);
    await vscode.commands.executeCommand("sweetpad.build.build");
    await waitFor(() => findFiles(output(), PRODUCT).some((p) => p !== debugProduct), {
      timeout: 180_000,
      message: "building Release produced no product distinct from the Debug one",
    });

    assert.ok(
      findFiles(output(), PRODUCT).includes(debugProduct),
      "the Release build overwrote or removed the Debug product",
    );
    await config().update("build.configuration", "Debug", vscode.ConfigurationTarget.Workspace);
  });

  test("cleaning removes what the build produced", async () => {
    assert.ok(findFile(output(), PRODUCT), "nothing to clean — the assertion would prove nothing");

    await vscode.commands.executeCommand("sweetpad.build.clean");

    await waitFor(() => findFile(output(), PRODUCT) === undefined, {
      timeout: 180_000,
      message: `${PRODUCT} survived a clean under ${output()}`,
    });
  });

  test("a build that cannot compile produces nothing and reports a failure", async () => {
    for (const stale of findFiles(output(), PRODUCT)) {
      rmSync(stale, { force: true });
    }
    writeFileSync(source(), `${pristine}\nvoid broken(void) { return undeclared_symbol_here; }\n`, "utf8");

    // A failing build has to be distinguishable from a successful one by
    // something other than the absence of a product — otherwise nothing
    // downstream (a task chain, a CI script, an agent) can tell them apart.
    // Today that signal is the task's exit status; if the pipeline ever stops
    // running builds as tasks, this assertion should be replaced deliberately
    // rather than deleted.
    const exits: number[] = [];
    const trace: string[] = [];
    let ended = false;
    const subscriptions = [
      vscode.tasks.onDidStartTask((e) => trace.push(`started ${e.execution.task.name}`)),
      vscode.tasks.onDidEndTaskProcess((e) => {
        trace.push(`process-exit ${e.execution.task.name} code=${e.exitCode}`);
        if (e.exitCode !== undefined) {
          exits.push(e.exitCode);
        }
      }),
      vscode.tasks.onDidEndTask((e) => {
        trace.push(`ended ${e.execution.task.name}`);
        ended = true;
      }),
    ];

    try {
      await vscode.commands.executeCommand("sweetpad.build.build");
      await waitFor(() => ended, {
        timeout: 60_000,
        message: `the failing build never ran a task to completion\ntask activity: ${trace.join(" | ") || "(none)"}`,
      });
      assert.equal(findFile(output(), PRODUCT), undefined, "a failing build still produced a product");
      assert.ok(
        exits.length > 0,
        `a failing build reported no exit status, so nothing downstream can tell it failed\ntask activity: ${trace.join(" | ")}`,
      );
      assert.ok(
        exits.every((code) => code !== 0),
        `a build that cannot compile reported success: exit codes ${exits.join(", ")}`,
      );
    } finally {
      for (const subscription of subscriptions) {
        subscription.dispose();
      }
      writeFileSync(source(), pristine, "utf8");
    }
  });
});
