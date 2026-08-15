import * as assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";

import * as vscode from "vscode";

import { activate, seedBuildSelection, waitFor, workspacePath } from "./helpers";

/**
 * Compile errors reach the Problems panel, and clear again once the code is
 * fixed. This is the strongest contract in the suite: a user judges a build by
 * the squiggles they get, and squiggles are observable no matter what produces
 * them — so it holds equally for `xcodebuild` today and sweetpad-core later.
 *
 * Deliberately says nothing about *how* a diagnostic is derived (parsed from a
 * log, read from a result bundle, streamed over a protocol). Only that building
 * broken code reports the break, at the right place, with the right severity.
 *
 * The two cases share one broken build: the error is introduced and built once,
 * then the second case fixes it and rebuilds. Builds are what this suite costs,
 * so it does the minimum number that still proves both directions.
 */
suite("diagnostics", () => {
  const source = () => workspacePath("widget.m");
  const uri = () => vscode.Uri.file(source());
  const errorsInSource = () =>
    vscode.languages.getDiagnostics(uri()).filter((d) => d.severity === vscode.DiagnosticSeverity.Error);

  let pristine: string;
  let brokenLine: number;

  suiteSetup(async function () {
    await activate();
    await seedBuildSelection();
    pristine = readFileSync(source(), "utf8");
    brokenLine = pristine.split("\n").length;

    writeFileSync(source(), `${pristine}\nvoid broken(void) { return undeclared_symbol_here; }\n`, "utf8");
    await vscode.commands.executeCommand("sweetpad.build.build");
    await waitFor(() => errorsInSource().length > 0, {
      timeout: 180_000,
      message: "a failing build reported no error diagnostic for the broken file",
    });
  });

  suiteTeardown(() => {
    writeFileSync(source(), pristine, "utf8");
  });

  test("a compile error is reported against the file that caused it", () => {
    const errors = errorsInSource();
    assert.ok(errors.length > 0, "expected at least one error");
    // The line the break is on, not merely "somewhere in the file".
    assert.ok(
      errors.some((d) => Math.abs(d.range.start.line - brokenLine) <= 2),
      `no diagnostic near line ${brokenLine}: ${errors.map((d) => `${d.range.start.line}:${d.message}`).join(" | ")}`,
    );
  });

  test("diagnostics clear once the code compiles again", async () => {
    writeFileSync(source(), pristine, "utf8");
    await vscode.commands.executeCommand("sweetpad.build.build");

    await waitFor(() => errorsInSource().length === 0, {
      timeout: 180_000,
      message: "errors remained after the code was fixed",
    });
  });
});
