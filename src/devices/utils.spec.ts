/**
 * Unit tests for supportsDevicectl utility function
 */

import { supportsDevicectl } from "./utils";

describe("supportsDevicectl", () => {
  describe("iOS 17+ support (default minimum version)", () => {
    it("returns true for iOS 17.0", () => {
      expect(supportsDevicectl("17.0", 17)).toBe(true);
    });

    it("returns true for iOS 17.1", () => {
      expect(supportsDevicectl("17.1", 17)).toBe(true);
    });

    it("returns true for iOS 18.0", () => {
      expect(supportsDevicectl("18.0", 17)).toBe(true);
    });

    it("returns true for iOS 18.5.1", () => {
      expect(supportsDevicectl("18.5.1", 17)).toBe(true);
    });

    it("returns false for iOS 16.7", () => {
      expect(supportsDevicectl("16.7", 17)).toBe(false);
    });

    it("returns false for iOS 16.6", () => {
      expect(supportsDevicectl("16.6", 17)).toBe(false);
    });

    it("returns false for iOS 15.0", () => {
      expect(supportsDevicectl("15.0", 17)).toBe(false);
    });
  });

  describe("beta versions", () => {
    it("returns true for iOS 17 beta", () => {
      expect(supportsDevicectl("17 beta", 17)).toBe(true);
    });

    it("returns true for iOS 17.0 beta 3", () => {
      expect(supportsDevicectl("17.0 beta 3", 17)).toBe(true);
    });

    it("returns true for iOS 18 beta 2", () => {
      expect(supportsDevicectl("18 beta 2", 17)).toBe(true);
    });

    it("returns false for iOS 16 beta", () => {
      expect(supportsDevicectl("16 beta", 17)).toBe(false);
    });
  });

  describe("edge cases", () => {
    it("returns false for undefined OS version", () => {
      expect(supportsDevicectl(undefined, 17)).toBe(false);
    });

    it("returns false for 'Unknown' OS version", () => {
      expect(supportsDevicectl("Unknown", 17)).toBe(false);
    });

    it("returns false for empty string", () => {
      expect(supportsDevicectl("", 17)).toBe(false);
    });

    it("returns false for malformed version string", () => {
      expect(supportsDevicectl("beta", 17)).toBe(false);
    });

    it("returns false for version string with no leading digits", () => {
      expect(supportsDevicectl(".0.1", 17)).toBe(false);
    });
  });

  describe("custom minimum versions", () => {
    it("returns true for watchOS 10+ with minVersion 10", () => {
      expect(supportsDevicectl("10.0", 10)).toBe(true);
      expect(supportsDevicectl("10.1", 10)).toBe(true);
      expect(supportsDevicectl("11.0", 10)).toBe(true);
    });

    it("returns false for watchOS < 10 with minVersion 10", () => {
      expect(supportsDevicectl("9.5", 10)).toBe(false);
      expect(supportsDevicectl("9.0", 10)).toBe(false);
      expect(supportsDevicectl("8.0", 10)).toBe(false);
    });

    it("returns true for tvOS 17+ with minVersion 17", () => {
      expect(supportsDevicectl("17.0", 17)).toBe(true);
      expect(supportsDevicectl("18.0", 17)).toBe(true);
    });

    it("returns false for tvOS < 17 with minVersion 17", () => {
      expect(supportsDevicectl("16.5", 17)).toBe(false);
      expect(supportsDevicectl("16.0", 17)).toBe(false);
    });

    it("returns true for visionOS 1+ with minVersion 1", () => {
      expect(supportsDevicectl("1.0", 1)).toBe(true);
      expect(supportsDevicectl("1.1", 1)).toBe(true);
      expect(supportsDevicectl("2.0", 1)).toBe(true);
    });

    it("handles beta versions for custom minimum versions", () => {
      expect(supportsDevicectl("10 beta", 10)).toBe(true);
      expect(supportsDevicectl("9 beta", 10)).toBe(false);
      expect(supportsDevicectl("1 beta", 1)).toBe(true);
    });
  });

  describe("single digit versions", () => {
    it("handles single digit major versions", () => {
      expect(supportsDevicectl("9", 10)).toBe(false);
      expect(supportsDevicectl("10", 10)).toBe(true);
      expect(supportsDevicectl("17", 17)).toBe(true);
    });
  });

  describe("version strings with extra spaces", () => {
    it("handles version strings with leading/trailing spaces", () => {
      // The function doesn't trim, so leading spaces would break the regex
      expect(supportsDevicectl(" 17.0", 17)).toBe(false);
      // But trailing spaces after the number are fine
      expect(supportsDevicectl("17.0 ", 17)).toBe(true);
    });
  });
});
