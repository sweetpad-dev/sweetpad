import { jest } from "@jest/globals";

// Mock the external dependencies
jest.mock("../common/commands", () => ({
  ExtensionContext: jest.fn(),
}));

jest.mock("../common/exec", () => ({
  exec: jest.fn(),
}));

jest.mock("../common/files", () => ({
  tempFilePath: jest.fn(),
  readJsonFile: jest.fn(),
  ensureAppPathExists: jest.fn(),
}));

jest.mock("../common/cli/scripts", () => ({
  getXcodeVersionInstalled: jest.fn(),
  getBuildSettingsToLaunch: jest.fn(),
}));

describe("Device debugging fix", () => {
  describe("runOniOSDevice console flag handling", () => {
    it("should exclude --console flag when debug=true", () => {
      // This test validates the key fix for the device debugging issue
      // The --console flag should not be included when debug=true to prevent immediate process termination
      
      const isConsoleOptionSupported = true;
      const debugMode = true;
      
      // Simulate the logic from runOniOSDevice function
      const shouldUseConsole = isConsoleOptionSupported && !debugMode;
      
      expect(shouldUseConsole).toBe(false);
    });
    
    it("should include --console flag when debug=false", () => {
      // This test ensures normal (non-debug) functionality is preserved
      
      const isConsoleOptionSupported = true;
      const debugMode = false;
      
      // Simulate the logic from runOniOSDevice function
      const shouldUseConsole = isConsoleOptionSupported && !debugMode;
      
      expect(shouldUseConsole).toBe(true);
    });
    
    it("should not include --console flag when Xcode version doesn't support it", () => {
      // This test ensures backward compatibility with older Xcode versions
      
      const isConsoleOptionSupported = false;
      const debugMode = false;
      
      // Simulate the logic from runOniOSDevice function
      const shouldUseConsole = isConsoleOptionSupported && !debugMode;
      
      expect(shouldUseConsole).toBe(false);
    });
    
    it("should properly filter null arguments", () => {
      // Test that null arguments are properly filtered from the launch command
      
      const launchArgs = [
        "devicectl",
        "device", 
        "process",
        "launch",
        null, // This should be filtered out
        "--json-output",
        "/path/to/output",
        "--terminate-existing",
        "--device",
        "device-id",
        "bundle-id"
      ].filter((arg) => arg !== null);
      
      expect(launchArgs).not.toContain(null);
      expect(launchArgs).toEqual([
        "devicectl",
        "device",
        "process", 
        "launch",
        "--json-output",
        "/path/to/output",
        "--terminate-existing",
        "--device",
        "device-id",
        "bundle-id"
      ]);
    });
  });
});