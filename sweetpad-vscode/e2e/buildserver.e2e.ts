import * as assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";

import * as vscode from "vscode";

import { activate, seedProjectSelection, waitFor, workspacePath } from "./helpers";

/**
 * `buildServer.json` generation, driven through the real command against a real
 * project. This is the file SourceKit-LSP reads to find a build server, and it
 * is silently skipped when a required field is missing — so "the command ran"
 * is not the assertion that matters; "the file it wrote is one SourceKit-LSP
 * would accept" is.
 */
suite("build server config", () => {
  /** SourceKit-LSP decodes all five or ignores the file entirely. */
  const REQUIRED_FIELDS = ["name", "version", "bspVersion", "languages", "argv"];

  suiteSetup(async function () {
    await activate();
    await seedProjectSelection();
    await vscode.workspace
      .getConfiguration("sweetpad")
      .update("buildServer.provider", "sweetpad", vscode.ConfigurationTarget.Workspace);
  });

  test("writes a config SourceKit-LSP would accept", async () => {
    const configPath = workspacePath("buildServer.json");
    await vscode.commands.executeCommand("sweetpad.build.generateBuildServerConfig");
    await waitFor(() => existsSync(configPath), {
      message: `buildServer.json was never written to ${configPath}`,
    });

    const config = JSON.parse(readFileSync(configPath, "utf8"));
    const missing = REQUIRED_FIELDS.filter((field) => config[field] === undefined || config[field] === null);
    assert.deepEqual(missing, [], `SourceKit-LSP would skip this file; missing: ${missing.join(", ")}`);

    assert.ok(Array.isArray(config.argv) && config.argv.length > 0, "argv is empty");
    // A launcher path that doesn't exist is the failure mode an extension
    // update produces, and it looks exactly like "autocomplete stopped working".
    assert.ok(existsSync(config.argv[0]), `argv[0] does not exist on disk: ${config.argv[0]}`);
    assert.ok(
      Array.isArray(config.languages) && config.languages.includes("swift"),
      `languages does not cover swift: ${JSON.stringify(config.languages)}`,
    );
  });

  test("the BSP doctor runs and reports on the active provider", async () => {
    // The doctor is the command users are pointed at when autocomplete breaks;
    // it must survive being run, whatever it concludes.
    await vscode.commands.executeCommand("sweetpad.bsp.doctor");
  });
});
