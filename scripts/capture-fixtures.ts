import { existsSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { join, resolve, basename } from "node:path";
import { tmpdir } from "node:os";
import { randomBytes } from "node:crypto";
import { execa } from "execa";

// ── Types ──────────────────────────────────────────────────────────────

interface CaptureResult {
  command: string;
  filename: string;
  status: "ok" | "skipped" | "failed";
  error?: string;
}

interface Manifest {
  xcodeVersion: string;
  timestamp: string;
  workspacePath: string;
  tag: string;
  captures: CaptureResult[];
}

// ── Helpers ────────────────────────────────────────────────────────────

async function run(command: string, args: string[]): Promise<string> {
  const result = await execa(command, args);
  return result.stdout;
}

function tmpPath(prefix: string): string {
  return join(tmpdir(), `sweetpad-${prefix}-${randomBytes(4).toString("hex")}`);
}

function writeCapture(outputDir: string, filename: string, content: string): void {
  writeFileSync(join(outputDir, filename), content, "utf-8");
  console.log(`  ✓ ${filename}`);
}

// ── Capture functions ──────────────────────────────────────────────────

async function getXcodeVersion(): Promise<string> {
  const raw = await run("xcodebuild", ["-version"]);
  // "Xcode 16.0\nBuild version 16A242d" → "16.0"
  const match = raw.match(/Xcode\s+([\d.]+)/);
  return match?.[1] ?? "unknown";
}

async function captureSimulators(outputDir: string): Promise<CaptureResult> {
  const cmd = "xcrun simctl list devices --json";
  try {
    const stdout = await run("xcrun", ["simctl", "list", "devices", "--json"]);
    writeCapture(outputDir, "simctl-devices.json", stdout);
    return { command: cmd, filename: "simctl-devices.json", status: "ok" };
  } catch (e: any) {
    console.log(`  ✗ simctl-devices.json (${e.message})`);
    return { command: cmd, filename: "simctl-devices.json", status: "failed", error: e.message };
  }
}

async function captureDevicectl(outputDir: string): Promise<CaptureResult> {
  const cmd = "xcrun devicectl list devices";
  const tmp = tmpPath("devicectl");
  try {
    await run("xcrun", ["devicectl", "list", "devices", "--json-output", tmp, "--timeout", "10"]);
    const content = readFileSync(tmp, "utf-8");
    writeCapture(outputDir, "devicectl-devices.json", content);
    return { command: cmd, filename: "devicectl-devices.json", status: "ok" };
  } catch (e: any) {
    console.log(`  ✗ devicectl-devices.json (${e.message})`);
    return { command: cmd, filename: "devicectl-devices.json", status: "skipped", error: e.message };
  } finally {
    try {
      rmSync(tmp);
    } catch {}
  }
}

async function captureXcodebuildList(outputDir: string, workspace: string): Promise<{ result: CaptureResult; schemes: string[] }> {
  const cmd = `xcodebuild -list -json -workspace ${workspace}`;
  try {
    const stdout = await run("xcodebuild", ["-list", "-json", "-workspace", workspace]);
    writeCapture(outputDir, "xcodebuild-list.json", stdout);

    // Extract schemes for downstream captures
    const parsed = JSON.parse(stdout);
    const schemes: string[] = parsed.workspace?.schemes ?? parsed.project?.schemes ?? [];
    return {
      result: { command: cmd, filename: "xcodebuild-list.json", status: "ok" },
      schemes,
    };
  } catch (e: any) {
    console.log(`  ✗ xcodebuild-list.json (${e.message})`);
    return {
      result: { command: cmd, filename: "xcodebuild-list.json", status: "failed", error: e.message },
      schemes: [],
    };
  }
}

async function captureBuildSettings(outputDir: string, workspace: string, scheme: string): Promise<CaptureResult> {
  const filename = `build-settings-${scheme}.json`;
  const cmd = `xcodebuild -showBuildSettings -json -scheme ${scheme}`;
  try {
    const stdout = await run("xcodebuild", [
      "-showBuildSettings", "-json", "-scheme", scheme, "-workspace", workspace,
    ]);
    writeCapture(outputDir, filename, stdout);
    return { command: cmd, filename, status: "ok" };
  } catch (e: any) {
    console.log(`  ✗ ${filename} (${e.message})`);
    return { command: cmd, filename, status: "failed", error: e.message };
  }
}

async function captureDestinations(outputDir: string, workspace: string, scheme: string): Promise<CaptureResult> {
  const filename = `destinations-${scheme}.txt`;
  const cmd = `xcodebuild -showdestinations -scheme ${scheme}`;
  try {
    const stdout = await run("xcodebuild", [
      "-showdestinations", "-scheme", scheme, "-workspace", workspace,
    ]);
    writeCapture(outputDir, filename, stdout);
    return { command: cmd, filename, status: "ok" };
  } catch (e: any) {
    console.log(`  ✗ ${filename} (${e.message})`);
    return { command: cmd, filename, status: "failed", error: e.message };
  }
}

// ── Main ───────────────────────────────────────────────────────────────

function usage(): never {
  console.log(`Usage: tsx scripts/capture-fixtures.ts <workspace-path> [--tag <name>]

Captures real Xcode CLI output and saves as test fixtures.

Arguments:
  workspace-path   Path to .xcworkspace, .xcodeproj/project.xcworkspace, or Package.swift
  --tag <name>     Name for this capture (default: xcode version)

Examples:
  tsx scripts/capture-fixtures.ts ./MyApp.xcworkspace
  tsx scripts/capture-fixtures.ts ./MyApp.xcworkspace --tag bug-123
  tsx scripts/capture-fixtures.ts ./MyApp.xcodeproj/project.xcworkspace --tag xcode16`);
  process.exit(1);
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length === 0 || args.includes("--help")) {
    usage();
  }

  const workspacePath = resolve(args[0]);
  if (!existsSync(workspacePath)) {
    console.error(`Error: workspace not found: ${workspacePath}`);
    process.exit(1);
  }

  const tagIndex = args.indexOf("--tag");
  const xcodeVersion = await getXcodeVersion();
  const tag = tagIndex !== -1 && args[tagIndex + 1] ? args[tagIndex + 1] : `xcode-${xcodeVersion}`;

  const outputDir = join(process.cwd(), "tests", "fixtures", "captured", tag);
  mkdirSync(outputDir, { recursive: true });

  console.log(`\nCapturing fixtures from: ${workspacePath}`);
  console.log(`Xcode version: ${xcodeVersion}`);
  console.log(`Output: ${outputDir}\n`);

  const captures: CaptureResult[] = [];

  // Xcode version
  writeCapture(outputDir, "xcode-version.txt", `Xcode ${xcodeVersion}`);
  captures.push({ command: "xcodebuild -version", filename: "xcode-version.txt", status: "ok" });

  // Simulators (always available)
  captures.push(await captureSimulators(outputDir));

  // Physical devices (may fail if no devices connected)
  captures.push(await captureDevicectl(outputDir));

  // Xcode workspace/project listing
  const { result: listResult, schemes } = await captureXcodebuildList(outputDir, workspacePath);
  captures.push(listResult);

  // Per-scheme captures
  for (const scheme of schemes) {
    captures.push(await captureBuildSettings(outputDir, workspacePath, scheme));
    captures.push(await captureDestinations(outputDir, workspacePath, scheme));
  }

  // Write manifest
  const manifest: Manifest = {
    xcodeVersion,
    timestamp: new Date().toISOString(),
    workspacePath,
    tag,
    captures,
  };
  writeFileSync(join(outputDir, "manifest.json"), JSON.stringify(manifest, null, 2), "utf-8");

  const ok = captures.filter((c) => c.status === "ok").length;
  const failed = captures.filter((c) => c.status === "failed").length;
  const skipped = captures.filter((c) => c.status === "skipped").length;
  console.log(`\nDone: ${ok} captured, ${skipped} skipped, ${failed} failed`);
  console.log(`Manifest: ${join(outputDir, "manifest.json")}`);
}

void main().catch((e) => {
  console.error("Fatal error:", e);
  process.exit(1);
});
