// Hand-edited so descriptions stay sentence-shaped. Read by meta.usage / meta.schema.
export type MethodSchema = {
  description: string;
  params?: Record<string, { type: string; required?: boolean; description?: string; enum?: string[] }>;
  returns?: string;
};

export const METHOD_CATALOG: Record<string, MethodSchema> = {
  // meta
  "meta.usage": {
    description: "List every RPC method with a one-line description.",
    returns: "{ methods: { method: string; description: string }[] }",
  },
  "meta.schema": {
    description: "Return the JSON schema for one method, or for all methods.",
    params: {
      method: { type: "string", description: "Method name; omit to return the full catalog." },
    },
    returns: "MethodSchema | Record<string, MethodSchema>",
  },
  "meta.version": {
    description: "Return the extension and protocol version this server speaks.",
    returns: "{ extensionVersion: string; protocolVersion: string }",
  },
  "meta.workspacePath": {
    description: "Return the absolute realpath of the workspace this server owns.",
    returns: "{ workspacePath: string }",
  },

  // scheme
  "scheme.list": {
    description: "List all Xcode schemes detected for the workspace.",
    returns: "{ schemes: SchemeEntity[] }",
  },
  "scheme.get": {
    description: "Return the currently selected scheme for build (or null).",
    returns: "{ scheme: SchemeEntity | null }",
  },
  "scheme.set": {
    description: "Persist the build scheme selection.",
    params: { name: { type: "string", required: true } },
    returns: "{ scheme: SchemeEntity }",
  },

  // destination
  "destination.list": {
    description: "List all destinations (simulators, devices, macOS).",
    returns: "{ destinations: DestinationEntity[] }",
  },
  "destination.get": {
    description: "Return the currently selected destination for build (or null).",
    returns: "{ destination: DestinationEntity | null }",
  },
  "destination.set": {
    description: "Persist the build destination selection.",
    params: { id: { type: "string", required: true } },
    returns: "{ destination: DestinationEntity }",
  },

  // build configuration (Xcode-level, e.g. Debug / Release).
  // Distinct from vscodeSettings.* (which manipulates the VS Code sweetpad.* setting space).
  "buildConfig.list": {
    description: "List Xcode build configurations (e.g. Debug, Release).",
    returns: "{ configurations: ConfigurationEntity[] }",
  },
  "buildConfig.get": {
    description: "Return the currently selected Xcode build configuration (or null).",
    returns: "{ configuration: ConfigurationEntity | null }",
  },
  "buildConfig.set": {
    description: "Persist the Xcode build configuration selection.",
    params: { name: { type: "string", required: true } },
    returns: "{ configuration: ConfigurationEntity }",
  },

  // state
  "state.get": {
    description: "One-shot snapshot: scheme + destination + configuration + running/latest build.",
    returns: "StateSnapshot",
  },

  // simulator
  "simulator.list": {
    description: "List simulators (with state and availability).",
    params: {
      state: { type: "string", enum: ["Booted", "Shutdown"], description: "Filter by state." },
      available: { type: "boolean", description: "Filter by isAvailable." },
    },
    returns: "{ simulators: SimulatorEntity[] }",
  },
  "simulator.start": {
    description: "Boot a simulator (xcrun simctl boot). Accepts udid or destination id.",
    params: { id: { type: "string", required: true } },
    returns: "{ booted: true; alreadyRunning: boolean; simulator: SimulatorEntity }",
  },
  "simulator.stop": {
    description: "Shut down a simulator (xcrun simctl shutdown). Accepts udid or destination id.",
    params: { id: { type: "string", required: true } },
    returns: "{ stopped: true; alreadyStopped: boolean; simulator: SimulatorEntity }",
  },
  "simulator.refresh": {
    description: "Re-scan simulators and return the fresh list.",
    returns: "{ simulators: SimulatorEntity[] }",
  },

  // build
  "build.start": {
    description:
      "Trigger a build/run/launch/test/clean. Returns the buildId immediately. Validates prerequisites (scheme, configuration, and — except for clean — destination) and errors fast with MISSING_PREREQUISITES if any are unset.",
    params: {
      command: { type: "string", required: true, enum: ["build", "run", "launch", "test", "clean"] },
      debug: { type: "boolean", description: "Pass debug: true to the underlying command." },
      caller: {
        type: "string",
        description: "Free-form label stored on the BuildEntity (also accepts SWEETPAD_CALLER env).",
      },
    },
    returns: "{ buildId: string }",
  },
  "build.stop": {
    description:
      "Terminate the currently running scheme task (build, run, launch, test, or clean). For launch/run sessions this also stops the running app — they share one task.",
    returns: "{ stopped: boolean; buildId: string | null }",
  },
  "build.wait": {
    description:
      "Briefly poll for a build to finish. Timeout is capped server-side (~30s) to keep agent loops responsive. Always returns a BuildEntity — check `status` to know if it's still running and poll again if needed.",
    params: {
      buildId: { type: "string", description: "Defaults to the latest build." },
      timeoutMs: {
        type: "number",
        description: "Defaults to 10000 (10s); capped at 30000 (30s).",
      },
    },
    returns: "BuildEntity",
  },
  "build.status": {
    description: "Return one build's current entity. Defaults to the latest build.",
    params: { buildId: { type: "string" } },
    returns: "BuildEntity",
  },
  "build.list": {
    description: "List recent persisted builds, newest first.",
    params: {
      limit: { type: "number", description: "Defaults to 10." },
    },
    returns: "{ builds: BuildEntity[] }",
  },
  "build.logs": {
    description: "Return the raw xcodebuild log for one build (inline).",
    params: { buildId: { type: "string" } },
    returns: "{ buildId: string; log: string }",
  },
  "build.diagnostics": {
    description: "Return structured diagnostics for one build.",
    params: { buildId: { type: "string" } },
    returns: "{ buildId: string; diagnostics: DiagnosticEntity[] }",
  },

  // scheme files (.xcscheme on disk)
  "scheme.reveal": {
    description: "Locate the .xcscheme file for a scheme and return its absolute path + raw XML.",
    params: { name: { type: "string", required: true } },
    returns: "{ name: string; path: string; xml: string; allPaths: string[] }",
  },

  // simulator app operations (xcrun simctl wrappers)
  "simulator.install": {
    description: "Install an .app bundle on a booted simulator.",
    params: {
      udid: { type: "string", required: true },
      appPath: { type: "string", required: true },
    },
    returns: "{ udid: string; appPath: string }",
  },
  "simulator.uninstall": {
    description: "Uninstall an app from a booted simulator.",
    params: {
      udid: { type: "string", required: true },
      bundleId: { type: "string", required: true },
    },
    returns: "{ udid: string; bundleId: string }",
  },
  "simulator.launchApp": {
    description: "Launch an installed app on a booted simulator. Env entries are forwarded via SIMCTL_CHILD_*.",
    params: {
      udid: { type: "string", required: true },
      bundleId: { type: "string", required: true },
      args: { type: "string[]", description: "Extra argv appended after the bundle id." },
      env: { type: "Record<string, string>", description: "Environment forwarded to the launched process." },
      waitForDebugger: { type: "boolean" },
    },
    returns: "{ udid: string; bundleId: string; pid: number | null }",
  },
  "simulator.terminateApp": {
    description: "Terminate a running app on a booted simulator.",
    params: {
      udid: { type: "string", required: true },
      bundleId: { type: "string", required: true },
    },
    returns: "{ udid: string; bundleId: string }",
  },
  "simulator.openUrl": {
    description: "Open a URL on a booted simulator (xcrun simctl openurl).",
    params: {
      udid: { type: "string", required: true },
      url: { type: "string", required: true },
    },
    returns: "{ udid: string; url: string }",
  },
  "simulator.screenshot": {
    description:
      "Capture a PNG screenshot from a booted simulator. Path defaults to <workspace>/sweetpad-screenshot.png.",
    params: {
      udid: { type: "string", required: true },
      path: { type: "string" },
    },
    returns: "{ udid: string; path: string }",
  },

  // device operations (xcrun devicectl wrappers)
  "device.install": {
    description: "Install an .app bundle on a physical iOS device via devicectl.",
    params: {
      deviceId: { type: "string", required: true },
      appPath: { type: "string", required: true },
    },
    returns: "{ deviceId: string; appPath: string }",
  },
  "device.launch": {
    description:
      "Launch an installed app on a physical device. Env entries are forwarded via DEVICECTL_CHILD_*. Defaults to terminating any pre-existing instance.",
    params: {
      deviceId: { type: "string", required: true },
      bundleId: { type: "string", required: true },
      args: { type: "string[]" },
      env: { type: "Record<string, string>" },
      terminateExisting: { type: "boolean", description: "Default true." },
    },
    returns: "{ deviceId: string; bundleId: string; pid: number | null }",
  },
  "device.terminate": {
    description: "Terminate a running app on a physical device.",
    params: {
      deviceId: { type: "string", required: true },
      bundleId: { type: "string", required: true },
    },
    returns: "{ deviceId: string; bundleId: string }",
  },

  // xcodebuild low-level lookups
  "buildSettings.get": {
    description:
      "Raw xcodebuild -showBuildSettings -json for one scheme/configuration/sdk. Defaults to the persisted scheme + configuration; pass keys[] to limit the returned dictionary.",
    params: {
      scheme: { type: "string" },
      configuration: { type: "string" },
      sdk: { type: "string" },
      xcworkspace: { type: "string" },
      keys: { type: "string[]" },
    },
    returns: "{ targets: { target: string; settings: Record<string, string> }[] }",
  },
  "xcodebuild.list": {
    description: "Raw xcodebuild -list -json output (schemes / targets / configurations).",
    params: { xcworkspace: { type: "string" } },
    returns: "{ output: unknown }",
  },
  "appPath.find": {
    description: "Locate the .app bundle for a scheme/configuration/sdk on disk.",
    params: {
      scheme: { type: "string" },
      configuration: { type: "string" },
      sdk: { type: "string" },
      xcworkspace: { type: "string" },
    },
    returns: "{ appPath: string; target: string }",
  },
  "derivedData.path": {
    description: "Effective sweetpad.build.derivedDataPath; null when the user hasn't overridden Xcode's default.",
    returns: "{ derivedDataPath: string | null }",
  },
  "bundleId.get": {
    description: "Resolve PRODUCT_BUNDLE_IDENTIFIER for a scheme/configuration/sdk.",
    params: {
      scheme: { type: "string" },
      configuration: { type: "string" },
      sdk: { type: "string" },
      xcworkspace: { type: "string" },
    },
    returns: "{ bundleIdentifier: string; target: string }",
  },

  // workspace selection
  "workspace.detect": {
    description: "Scan the workspace tree for .xcworkspace / .xcodeproj / Package.swift candidates.",
    params: { depth: { type: "number", description: "Search depth, capped at 6." } },
    returns: "{ workspacePath: string; current: string | undefined; candidates: { path: string; kind: string }[] }",
  },
  "workspace.use": {
    description: "Override the active Xcode workspace via workspace state — same slot the QuickPick fills.",
    params: { path: { type: "string", required: true } },
    returns: "{ workspacePath: string; recent: string[] }",
  },
  "workspace.recent": {
    description: "Last ten workspaces set via workspace.use, newest first.",
    returns: "{ recent: string[] }",
  },

  // workspaceState (raw sweetpad.* keys in vscode.ExtensionContext.workspaceState)
  "workspaceState.get": {
    description: "Read one sweetpad.<key> from workspace state. The sweetpad. prefix is implicit.",
    params: { key: { type: "string", required: true } },
    returns: "{ key: string; value: unknown }",
  },
  "workspaceState.set": {
    description: "Write one sweetpad.<key>; pass value: null to clear.",
    params: {
      key: { type: "string", required: true },
      value: { type: "unknown" },
    },
    returns: "{ key: string; value: unknown }",
  },
  "workspaceState.keys": {
    description: "Every sweetpad.<key> currently set, with the prefix stripped.",
    returns: "{ keys: string[] }",
  },
  "workspaceState.delete": {
    description: "Remove one sweetpad.<key>.",
    params: { key: { type: "string", required: true } },
    returns: "{ key: string; deleted: boolean }",
  },

  // VS Code passthroughs
  "vscode.executeCommand": {
    description:
      "Pass-through to vscode.commands.executeCommand — invoke any registered command. Use to drive QuickPicks, debug actions, or commands contributed by other extensions.",
    params: {
      command: { type: "string", required: true },
      args: { type: "unknown[]" },
    },
    returns: "{ result: unknown }",
  },
  "vscodeSettings.get": {
    description: "Read one sweetpad.<key> setting (effective value).",
    params: { key: { type: "string", required: true } },
    returns: "{ key: string; value: unknown }",
  },
  "vscodeSettings.set": {
    description: "Write one sweetpad.<key> at the given target (default: workspace).",
    params: {
      key: { type: "string", required: true },
      value: { type: "unknown" },
      target: { type: "string", enum: ["global", "workspace", "workspaceFolder"] },
    },
    returns: "{ key: string; value: unknown; target: string }",
  },
  "vscodeSettings.inspect": {
    description: "Per-source view of a setting (default, global, workspace, workspaceFolder) plus the effective value.",
    params: { key: { type: "string", required: true } },
    returns:
      "{ key: string; default: unknown; global: unknown; workspace: unknown; workspaceFolder: unknown; effective: unknown }",
  },
  "vscodeSettings.list": {
    description: "Every sweetpad.<key> declared in package.json with its current effective value.",
    returns: "{ settings: { key: string; value: unknown }[] }",
  },

  // logs
  "logs.tail": {
    description: "Last N entries from the SweetPad output channel, optionally filtered by level.",
    params: {
      lines: { type: "number", description: "Defaults to 50, capped at 1000." },
      level: { type: "string", enum: ["debug", "info", "warning", "error"] },
    },
    returns: "{ count: number; entries: { time: string; level: string; message: string }[] }",
  },
};

export type MethodName = keyof typeof METHOD_CATALOG;
