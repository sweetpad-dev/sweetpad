import * as assert from "node:assert/strict";

import * as vscode from "vscode";

import { EXTENSION_ID, activate } from "./helpers";

/**
 * Settings as VS Code actually resolves them. The unit tests read configuration
 * through a stub, so a `package.json` default that disagrees with the code's
 * own fallback — or a key the code reads but nothing contributes — reads as
 * correct there and wrong in a real window.
 */
suite("configuration", () => {
  suiteSetup(async () => {
    await activate();
  });

  test("every contributed default is the value the API returns", () => {
    const extension = vscode.extensions.getExtension(EXTENSION_ID);
    const contributed = extension?.packageJSON.contributes.configuration;
    const sections: Record<string, { default?: unknown }>[] = (
      Array.isArray(contributed) ? contributed : [contributed]
    ).map((c: { properties: Record<string, { default?: unknown }> }) => c.properties);

    const mismatches: string[] = [];
    for (const properties of sections) {
      for (const [key, schema] of Object.entries(properties ?? {})) {
        if (!("default" in schema)) {
          continue;
        }
        const [, ...rest] = key.split(".");
        // `inspect().defaultValue`, not `get()`: other suites in this run write
        // workspace settings, and the assertion is about what the extension
        // contributes as its default, not what happens to be effective now.
        const actual = vscode.workspace.getConfiguration("sweetpad").inspect(rest.join("."))?.defaultValue;
        if (JSON.stringify(actual) !== JSON.stringify(schema.default)) {
          mismatches.push(`${key}: contributed ${JSON.stringify(schema.default)}, resolved ${JSON.stringify(actual)}`);
        }
      }
    }
    assert.deepEqual(mismatches, [], mismatches.join("\n"));
  });

  test("the build server provider resolves to a supported value", () => {
    // Which one is the default is a product decision; that it is one the code
    // knows how to route is not.
    const provider = vscode.workspace.getConfiguration("sweetpad").get<string>("buildServer.provider");
    assert.ok(
      provider === "sweetpad" || provider === "xcode-build-server",
      `unroutable build server provider: ${provider}`,
    );
  });
});
