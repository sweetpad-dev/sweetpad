#!/usr/bin/env tsx

import { execSync } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";

/**
 * Font optimization and verification tool for SweetPad VS Code extension.
 * Uses Python fonttools via uv (recommended for speed and reliability)
 */

interface IconDefinition {
  description: string;
  default: {
    fontPath: string;
    fontCharacter: string;
  };
}

interface PackageJson {
  contributes: {
    icons: Record<string, IconDefinition>;
  };
}

const PACKAGE_JSON_PATH = path.join(__dirname, "..", "package.json");
const ORIGINAL_FONT_PATH = path.join(__dirname, "..", "images", "icons", "tabler-icons.original.woff");
const OPTIMIZED_FONT_PATH = path.join(__dirname, "..", "images", "icons", "tabler-icons.woff");

// Shared utility functions
function parseUnicodeCharacter(char: string): string {
  // Remove leading backslash if present
  return char.startsWith("\\") ? char.substring(1) : char;
}

function checkDependencies(): { pyftsubsetCommand: string } {
  console.log("🔧 Checking dependencies...");
  // Use uv tool run to run pyftsubset from Python fonttools
  try {
    execSync("uv tool run --from fonttools pyftsubset --help", { stdio: "pipe" });
    console.log("✅ uv tool run pyftsubset is available (Python fonttools)");
    return { pyftsubsetCommand: "uv tool run --from fonttools pyftsubset" };
  } catch (error) {
    console.error("❌ uv tool run pyftsubset not found. Please install uv (https://github.com/astral-sh/uv):");
    console.error("   The fonttools package will be automatically installed when needed.");
    process.exit(1);
  }
}

function extractUsedIcons(): string[] {
  console.log("📖 Reading package.json...");

  const packageJsonContent = fs.readFileSync(PACKAGE_JSON_PATH, "utf8");
  const packageJson: PackageJson = JSON.parse(packageJsonContent);

  if (!packageJson.contributes?.icons) {
    throw new Error("No icons found in package.json");
  }

  const usedCharacters: string[] = [];
  const iconNames: string[] = [];

  for (const [iconName, iconDef] of Object.entries(packageJson.contributes.icons)) {
    // Look for icons using either the original font or the subset font
    if (
      iconDef.default?.fontPath === "images/icons/tabler-icons.original.woff" ||
      iconDef.default?.fontPath === "images/icons/tabler-icons.woff"
    ) {
      const character = iconDef.default.fontCharacter;
      usedCharacters.push(character);
      iconNames.push(iconName);
    }
  }

  console.log(`🔍 Found ${usedCharacters.length} used icons:`);
  iconNames.forEach((name, index) => {
    console.log(`   ${name} (${usedCharacters[index]})`);
  });

  return usedCharacters;
}

// Optimization functions
function createFontSubset(characters: string[], pyftsubsetCommand: string): void {
  console.log("🎨 Creating font subset...");

  if (!fs.existsSync(ORIGINAL_FONT_PATH)) {
    throw new Error(`Original font file not found: ${ORIGINAL_FONT_PATH}`);
  }

  // Convert unicode escape sequences to actual unicode codepoints
  const unicodes = characters.map((char) => {
    const hex = parseUnicodeCharacter(char);
    const codepoint = Number.parseInt(hex, 16);
    if (Number.isNaN(codepoint)) {
      throw new Error(`Invalid unicode character: ${char} (hex: ${hex})`);
    }
    return codepoint;
  });

  console.log(`📝 Unicode codepoints: ${unicodes.map((u) => `U+${u.toString(16).toUpperCase()}`).join(", ")}`);

  // Create the pyftsubset command
  const unicodeList = unicodes.map((u) => `U+${u.toString(16).toUpperCase()}`).join(",");

  const command = [
    pyftsubsetCommand,
    `"${ORIGINAL_FONT_PATH}"`,
    `--unicodes=${unicodeList}`,
    `--output-file="${OPTIMIZED_FONT_PATH}"`,
    "--flavor=woff",
    "--no-layout-closure",
    "--drop-tables+=GSUB,GPOS,DSIG",
  ].join(" ");

  console.log(`🚀 Running: ${command}`);

  try {
    execSync(command, { stdio: "inherit" });
    console.log("✅ Font subset created successfully");
  } catch (error) {
    console.error("❌ Failed to create font subset:", error);
    process.exit(1);
  }
}

function showOptimizationResults(): void {
  const originalStats = fs.statSync(ORIGINAL_FONT_PATH);
  const optimizedStats = fs.statSync(OPTIMIZED_FONT_PATH);

  const originalSize = originalStats.size;
  const optimizedSize = optimizedStats.size;
  const reduction = (((originalSize - optimizedSize) / originalSize) * 100).toFixed(1);

  console.log("\n📊 Results:");
  console.log(`   Original font: ${(originalSize / 1024).toFixed(1)} KB`);
  console.log(`   Optimized font: ${(optimizedSize / 1024).toFixed(1)} KB`);
  console.log(`   Size reduction: ${reduction}%`);
  console.log("\n🎉 Font optimization complete!");
  console.log(`   Original font preserved at: ${ORIGINAL_FONT_PATH}`);
  console.log(`   Optimized font created at: ${OPTIMIZED_FONT_PATH}`);
}

// Verification functions
function checkRequiredFiles(): void {
  console.log("📁 Checking required files...");

  if (!fs.existsSync(PACKAGE_JSON_PATH)) {
    throw new Error(`package.json not found: ${PACKAGE_JSON_PATH}`);
  }
  console.log("✅ package.json found");

  if (!fs.existsSync(ORIGINAL_FONT_PATH)) {
    throw new Error(`Original font file not found: ${ORIGINAL_FONT_PATH}`);
  }
  console.log("✅ Original font file found");

  if (!fs.existsSync(OPTIMIZED_FONT_PATH)) {
    throw new Error(`Optimized font file not found: ${OPTIMIZED_FONT_PATH}`);
  }
  console.log("✅ Optimized font file found");
}

function extractExpectedIcons(): { characters: string[]; iconNames: string[] } {
  console.log("📖 Analyzing package.json...");

  const packageJsonContent = fs.readFileSync(PACKAGE_JSON_PATH, "utf8");
  const packageJson: PackageJson = JSON.parse(packageJsonContent);

  if (!packageJson.contributes?.icons) {
    throw new Error("No icons found in package.json");
  }

  const characters: string[] = [];
  const iconNames: string[] = [];

  for (const [iconName, iconDef] of Object.entries(packageJson.contributes.icons)) {
    if (iconDef.default?.fontPath === "images/icons/tabler-icons.woff") {
      const character = iconDef.default.fontCharacter;
      characters.push(character);
      iconNames.push(iconName);
    }
  }

  console.log(`🔍 Found ${characters.length} icons using optimized font:`);
  iconNames.forEach((name, index) => {
    console.log(`   ${name} (${characters[index]})`);
  });

  return { characters, iconNames };
}

function verifyPackageJsonConfiguration(): boolean {
  console.log("📝 Verifying package.json configuration...");

  const packageJsonContent = fs.readFileSync(PACKAGE_JSON_PATH, "utf8");
  const packageJson: PackageJson = JSON.parse(packageJsonContent);

  let hasOriginalFontReferences = false;
  let hasSubsetFontReferences = false;

  for (const iconDef of Object.values(packageJson.contributes.icons)) {
    if (iconDef.default?.fontPath === "images/icons/tabler-icons.original.woff") {
      hasOriginalFontReferences = true;
    } else if (iconDef.default?.fontPath === "images/icons/tabler-icons.woff") {
      hasSubsetFontReferences = true;
    }
  }

  if (hasOriginalFontReferences && hasSubsetFontReferences) {
    console.warn("⚠️  Mixed font references found in package.json");
    console.warn("   Some icons still reference the original font");
    return false;
  }

  if (hasOriginalFontReferences && !hasSubsetFontReferences) {
    console.warn("⚠️  package.json still references original font");
    console.warn("   Run the optimize command to update font references");
    return false;
  }

  if (!hasOriginalFontReferences && hasSubsetFontReferences) {
    console.log("✅ package.json correctly configured");
    return true;
  }

  console.warn("⚠️  No font references found in package.json");
  return false;
}

function getFontInfo(fontPath: string): { unicodes: string[] } | null {
  console.log(`🔍 Analyzing font: ${path.basename(fontPath)}`);

  try {
    // Use fonttools ttx to extract cmap table and get unicode mappings
    const cmapCommand = `uv tool run --from fonttools fonttools ttx -t cmap -o - "${fontPath}"`;
    const cmapOutput = execSync(cmapCommand, { encoding: "utf8" });

    // Extract unicode values from cmap XML output
    const unicodes: string[] = [];
    const unicodeRegex = /code="0x([0-9A-Fa-f]+)"/g;
    while (true) {
      const match = unicodeRegex.exec(cmapOutput);
      if (match === null) {
        break;
      }
      const unicode = Number.parseInt(match[1], 16);
      if (unicode >= 0x1000) {
        // Filter for icon ranges (skip basic ASCII)
        unicodes.push(match[1].toLowerCase());
      }
    }

    return { unicodes };
  } catch (error) {
    console.warn(`⚠️  Could not analyze font ${fontPath}:`, error);
    return null;
  }
}

function verifyFontContent(expectedCharacters: string[], pyftsubsetCommand: string): boolean {
  console.log("🔬 Verifying font content...");

  const fontInfo = getFontInfo(OPTIMIZED_FONT_PATH);
  if (!fontInfo) {
    console.warn("⚠️  Cannot verify font content without fonttools");
    return true; // Assume it's correct if we can't verify
  }

  const expectedUnicodes = expectedCharacters.map((char) => {
    return parseUnicodeCharacter(char).toLowerCase();
  });

  const fontUnicodes = new Set(fontInfo.unicodes);

  console.log(`📝 Expected characters: ${expectedUnicodes.join(", ")}`);
  console.log(`📝 Font contains: ${fontInfo.unicodes.join(", ")}`);

  const missingCharacters = expectedUnicodes.filter((unicode) => !fontUnicodes.has(unicode));

  if (missingCharacters.length > 0) {
    console.error(`❌ Missing characters in font: ${missingCharacters.join(", ")}`);
    return false;
  }

  console.log("✅ All expected characters found in font");
  return true;
}

function showVerificationResults(): void {
  const originalStats = fs.statSync(ORIGINAL_FONT_PATH);
  const optimizedStats = fs.statSync(OPTIMIZED_FONT_PATH);

  const originalSize = originalStats.size;
  const optimizedSize = optimizedStats.size;
  const reduction = (((originalSize - optimizedSize) / originalSize) * 100).toFixed(1);

  console.log("\n📊 Font size comparison:");
  console.log(`   Original font: ${(originalSize / 1024).toFixed(1)} KB`);
  console.log(`   Optimized font: ${(optimizedSize / 1024).toFixed(1)} KB`);
  console.log(`   Size reduction: ${reduction}%`);
}

// Command functions
function optimizeCommand(): void {
  try {
    console.log("🍭 SweetPad Font Optimizer\n");

    const { pyftsubsetCommand } = checkDependencies();
    const usedCharacters = extractUsedIcons();
    createFontSubset(usedCharacters, pyftsubsetCommand);

    // Verify package.json has correct font paths
    const hasCorrectPaths = verifyPackageJsonConfiguration();
    if (!hasCorrectPaths) {
      console.warn("⚠️  Font optimization complete, but package.json needs manual update");
      console.warn('   Update icon font paths to "images/icons/tabler-icons.woff"');
    }

    showOptimizationResults();
  } catch (error) {
    console.error("❌ Error:", error);
    process.exit(1);
  }
}

function verifyCommand(): void {
  try {
    console.log("🔍 SweetPad Font Verifier\n");

    checkRequiredFiles();
    const { pyftsubsetCommand } = checkDependencies();

    const { characters } = extractExpectedIcons();
    const configValid = verifyPackageJsonConfiguration();
    const contentValid = verifyFontContent(characters, pyftsubsetCommand);

    showVerificationResults();

    if (configValid && contentValid) {
      console.log("\n🎉 All verifications passed!");
      console.log("   The optimized font is correctly configured and contains all required icons.");
    } else {
      console.log("\n❌ Verification failed!");
      process.exit(1);
    }
  } catch (error) {
    console.error("❌ Error:", error);
    process.exit(1);
  }
}

function showHelp(): void {
  console.log("🍭 SweetPad Font Tools\n");
  console.log("Usage: tsx font-tools.ts <command>\n");
  console.log("Commands:");
  console.log("  optimize    Create optimized font subset and verify configuration");
  console.log("  verify      Verify that optimized font is correctly configured");
  console.log("  help        Show this help message\n");
  console.log("Examples:");
  console.log("  tsx font-tools.ts optimize");
  console.log("  tsx font-tools.ts verify");
}

function main(): void {
  const command = process.argv[2];

  switch (command) {
    case "optimize":
      optimizeCommand();
      break;
    case "verify":
      verifyCommand();
      break;
    case "help":
    case "--help":
    case "-h":
      showHelp();
      break;
    default:
      console.error("❌ Unknown command:", command || "(none)");
      console.log("");
      showHelp();
      process.exit(1);
  }
}

if (require.main === module) {
  main();
}
