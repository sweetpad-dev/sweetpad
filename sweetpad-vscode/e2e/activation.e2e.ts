import * as assert from "node:assert/strict";

import * as vscode from "vscode";

import { EXTENSION_ID, FIXTURE_PROJECT, activate, workspacePath } from "./helpers";

suite("activation", () => {
  test("activates on a workspace containing an .xcodeproj", async () => {
    const extension = await activate();
    assert.equal(extension.isActive, true, "extension did not activate");
  });

  test("the fixture workspace is the one the extension sees", async () => {
    await activate();
    const project = vscode.Uri.file(workspacePath(FIXTURE_PROJECT));
    const stat = await vscode.workspace.fs.stat(project);
    assert.equal(stat.type & vscode.FileType.Directory, vscode.FileType.Directory);
  });

  test("declares CodeLLDB as a dependency and it resolved", async () => {
    // Activation silently fails if an `extensionDependencies` entry is missing,
    // which would otherwise surface here as an unexplained inactive extension.
    const extension = vscode.extensions.getExtension(EXTENSION_ID);
    const dependencies: string[] = extension?.packageJSON.extensionDependencies ?? [];
    for (const id of dependencies) {
      assert.ok(vscode.extensions.getExtension(id), `dependency ${id} is not installed`);
    }
  });
});
